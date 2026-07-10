//! Typed AST views over the lossless CST.
//!
//! Every type in this module is a thin, cheaply-cloneable wrapper around a
//! [`SyntaxNode`]: an AST node *projects* structure onto the underlying
//! red/green tree without ever owning text or mutating the tree. Casting is
//! infallible-or-`None` (a wrapper only exists when the node has the matching
//! [`crate::SyntaxKind`]), and dropping a wrapper drops nothing but an
//! `Arc` handle.
//!
//! ## Where engine quirks live
//!
//! The reference TyranoScript engine (`kag.parser.js`) *destroys* information
//! while parsing: it trims lines, drops escape backslashes, strips quotes,
//! removes or trims spaces in parameter values, truncates `*label|a|b` and
//! `#name:a:b` at the first separator, and coerces a literal `undefined` to
//! the empty string. The CST deliberately keeps all of that raw text. The
//! engine's lossy interpretation is reproduced here as **cooked value**
//! computations —
//! [`ParamValue::cooked`], [`TextLine::cooked_text`], [`LabelLine::value`],
//! [`CharaLine::face`], and friends — parameterised by [`InterpretOptions`]
//! so callers can pick engine-compatible or stricter behaviour. Nothing in
//! this module changes the tree; two calls with different options read the
//! same nodes and cook differently.
//!
//! The ground truth for the cooking rules is the engine-compatible lexer in
//! the `tyrano-lexer` crate (`read_param_value`, `finalize_param_value`,
//! `lex_label_line`, `lex_chara_line`, `lex_text_content`); this module ports
//! those behaviours faithfully, operating on raw CST token text instead of
//! during lexing.

use crate::kind::SyntaxKind;
use crate::red::{SyntaxNode, SyntaxToken};
use crate::text::TextSize;

// ======================================================================
// Interpretation options
// ======================================================================

/// How many spaces survive inside a cooked parameter value.
///
/// Mirrors TyranoScript's `KeepSpaceInParameterValue` engine setting
/// (values 1/2/3).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KeepSpaceLevel {
    /// Level 1: strip **every** half-width space while reading the value
    /// (backquote-delimited values are exempt).
    RemoveAll,
    /// Level 2 (engine default): keep interior spaces but trim both ends.
    TrimEnds,
    /// Level 3: keep the value exactly as written.
    KeepAll,
}

/// Engine-quirk interpretation options. The defaults reproduce the reference
/// engine's behaviour exactly.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InterpretOptions {
    /// Whitespace handling for cooked parameter values.
    pub keep_space: KeepSpaceLevel,
    /// When set, a label value `a|b|c` is truncated to its first `|` segment
    /// (`a`), matching the engine's `split("|")[1]`.
    pub label_value_first_segment_only: bool,
    /// When set, a character face `a:b:c` is truncated to its first `:`
    /// segment (`a`), matching the engine's `split(":")[1]`.
    pub chara_face_first_segment_only: bool,
}

impl Default for InterpretOptions {
    /// Engine-compatible defaults: [`KeepSpaceLevel::TrimEnds`] and both
    /// first-segment truncations enabled.
    fn default() -> Self {
        InterpretOptions {
            keep_space: KeepSpaceLevel::TrimEnds,
            label_value_first_segment_only: true,
            chara_face_first_segment_only: true,
        }
    }
}

// ======================================================================
// AstNode trait + macro
// ======================================================================

/// A typed view over a syntax node of a fixed [`SyntaxKind`].
pub trait AstNode: Sized {
    /// Whether a node of `kind` can be viewed as this type.
    fn can_cast(kind: SyntaxKind) -> bool;
    /// Wraps `node` if its kind matches, otherwise returns `None`.
    fn cast(node: SyntaxNode) -> Option<Self>;
    /// The underlying syntax node.
    fn syntax(&self) -> &SyntaxNode;
}

/// Generates a newtype AST wrapper plus its [`AstNode`] impl for a single
/// [`SyntaxKind`], in the style of rust-analyzer's `ast_node!`.
macro_rules! ast_node {
    ($(#[$meta:meta])* $name:ident, $kind:ident) => {
        $(#[$meta])*
        #[derive(Debug, Clone)]
        pub struct $name {
            syntax: SyntaxNode,
        }

        impl AstNode for $name {
            fn can_cast(kind: SyntaxKind) -> bool {
                kind == SyntaxKind::$kind
            }

            fn cast(node: SyntaxNode) -> Option<Self> {
                if Self::can_cast(node.kind()) {
                    Some($name { syntax: node })
                } else {
                    None
                }
            }

            fn syntax(&self) -> &SyntaxNode {
                &self.syntax
            }
        }
    };
}

// ======================================================================
// Small tree-walking helpers (never panic on malformed trees)
// ======================================================================

/// The first present (non-missing) direct child token of the given kind.
fn child_token(node: &SyntaxNode, kind: SyntaxKind) -> Option<SyntaxToken> {
    node.children_with_tokens()
        .filter_map(|e| e.into_token())
        .find(|t| t.kind() == kind && !t.is_missing())
}

/// Whether the node has a present direct child token of the given kind.
fn has_token(node: &SyntaxNode, kind: SyntaxKind) -> bool {
    child_token(node, kind).is_some()
}

/// The first direct child node castable to `N`.
fn child<N: AstNode>(node: &SyntaxNode) -> Option<N> {
    node.children().find_map(N::cast)
}

/// All direct child nodes castable to `N`, in order.
fn children<N: AstNode>(node: &SyntaxNode) -> Vec<N> {
    node.children().filter_map(N::cast).collect()
}

/// Text of the first present TEXT token inside the first child node of
/// `wrapper` kind (used for LABEL_NAME / LABEL_VALUE / CHARA_NAME /
/// CHARA_FACE, each of which wraps a single TEXT token).
fn wrapped_text(node: &SyntaxNode, wrapper: SyntaxKind) -> Option<String> {
    let inner = node.children().find(|n| n.kind() == wrapper)?;
    child_token(&inner, SyntaxKind::TEXT).map(|t| t.text().to_string())
}

/// Resolves `\` escapes in raw text: the backslash is dropped and the next
/// character kept verbatim; a trailing lone backslash is dropped. Mirrors the
/// engine's `lex_text_content` escape flag.
fn resolve_escapes(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut chars = s.chars();
    while let Some(c) = chars.next() {
        if c == '\\' {
            if let Some(next) = chars.next() {
                out.push(next);
            }
            // A trailing lone backslash is silently dropped.
        } else {
            out.push(c);
        }
    }
    out
}

/// The core parameter-value cook, ported from the engine's
/// `read_param_value` + `finalize_param_value`.
///
/// `raw` is the value token's verbatim text (quotes included for quoted
/// values). The steps:
///
/// 1. Detect a leading quote (`"`, `'`, or `` ` ``); if present, scanning
///    starts just past it and the same quote acts as the closing delimiter.
/// 2. Resolve `\` escapes (drop the backslash, keep the escaped char; a
///    trailing lone backslash is dropped).
/// 3. At [`KeepSpaceLevel::RemoveAll`] blank out every space *before* the
///    escape check, so even `\ ` vanishes — unless the value was
///    backquote-quoted, which is exempt.
/// 4. Finalize: trim both ends (unless [`KeepSpaceLevel::KeepAll`]).
/// 5. A value whose trimmed form is exactly `undefined` becomes `""`.
fn cook_value(raw: &str, opts: &InterpretOptions) -> String {
    let chars: Vec<char> = raw.chars().collect();
    let quote = chars.first().copied().filter(|c| matches!(c, '"' | '\'' | '`'));
    let (start, end_char) = match quote {
        Some(q) => (1usize, q),
        None => (0usize, ' '),
    };
    let remove_spaces =
        opts.keep_space == KeepSpaceLevel::RemoveAll && quote != Some('`');

    let mut value = String::new();
    let mut escape = false;
    let mut i = start;
    while i < chars.len() {
        let mut c = chars[i];
        if c == end_char && !escape {
            // The closing delimiter (or, for unquoted values, the first
            // unescaped space) ends the value; anything after is ignored.
            break;
        }
        // The engine blanks spaces (level 1) *before* the escape check, so an
        // escaped space disappears at that level too.
        if remove_spaces && c == ' ' {
            c = '\0';
        }
        if escape {
            if c != '\0' {
                value.push(c);
            }
            escape = false;
        } else if c == '\\' {
            escape = true;
        } else if c != '\0' {
            value.push(c);
        }
        i += 1;
    }

    // finalize_param_value: trim, then undefined-coercion.
    let trimmed = value.trim();
    if trimmed == "undefined" {
        return String::new();
    }
    if opts.keep_space == KeepSpaceLevel::KeepAll {
        value
    } else {
        trimmed.to_string()
    }
}

// ======================================================================
// Scenario + Line
// ======================================================================

ast_node!(
    /// The root node: a whole `.ks` scenario file.
    Scenario,
    SCENARIO
);

impl Scenario {
    /// Views the syntax root of a [`crate::parser::Parse`] as a scenario.
    ///
    /// Infallible for trees produced by this crate's parser (the root is
    /// always `SCENARIO`).
    pub fn from_parse(parse: &crate::parser::Parse) -> Scenario {
        Scenario::cast(parse.syntax()).expect("parser root is always SCENARIO")
    }

    /// The line-level constructs of the scenario, in order. Bare blank-line
    /// newline tokens and the trailing EOF token are skipped (only nodes are
    /// yielded).
    pub fn lines(&self) -> impl Iterator<Item = Line> + '_ {
        self.syntax.children().filter_map(Line::cast)
    }
}

/// One line-level construct directly under a [`Scenario`].
#[derive(Debug, Clone)]
pub enum Line {
    /// A free-text line.
    Text(TextLine),
    /// A `*name|value` label line.
    Label(LabelLine),
    /// A `#name:face` character line.
    Chara(CharaLine),
    /// A `;comment` line.
    Comment(CommentLine),
    /// A `/* … */` block comment.
    BlockComment(BlockComment),
    /// An `@tag …` whole-line tag.
    AtTag(AtTagLine),
    /// An `[iscript] … [endscript]` block.
    IScript(IScriptBlock),
    /// A `[html] … [endhtml]` block.
    Html(HtmlBlock),
    /// A syntactically invalid region.
    Error(ErrorLine),
}

impl Line {
    /// Casts a scenario child node to the matching line variant, or `None`
    /// for a non-line node.
    pub fn cast(node: SyntaxNode) -> Option<Line> {
        match node.kind() {
            SyntaxKind::TEXT_LINE => TextLine::cast(node).map(Line::Text),
            SyntaxKind::LABEL_LINE => LabelLine::cast(node).map(Line::Label),
            SyntaxKind::CHARA_LINE => CharaLine::cast(node).map(Line::Chara),
            SyntaxKind::COMMENT_LINE => CommentLine::cast(node).map(Line::Comment),
            SyntaxKind::BLOCK_COMMENT => BlockComment::cast(node).map(Line::BlockComment),
            SyntaxKind::AT_TAG_LINE => AtTagLine::cast(node).map(Line::AtTag),
            SyntaxKind::ISCRIPT_BLOCK => IScriptBlock::cast(node).map(Line::IScript),
            SyntaxKind::HTML_BLOCK => HtmlBlock::cast(node).map(Line::Html),
            SyntaxKind::ERROR => ErrorLine::cast(node).map(Line::Error),
            _ => None,
        }
    }

    /// The underlying syntax node of whichever variant this is.
    pub fn syntax(&self) -> &SyntaxNode {
        match self {
            Line::Text(n) => n.syntax(),
            Line::Label(n) => n.syntax(),
            Line::Chara(n) => n.syntax(),
            Line::Comment(n) => n.syntax(),
            Line::BlockComment(n) => n.syntax(),
            Line::AtTag(n) => n.syntax(),
            Line::IScript(n) => n.syntax(),
            Line::Html(n) => n.syntax(),
            Line::Error(n) => n.syntax(),
        }
    }
}

// ======================================================================
// TextLine
// ======================================================================

ast_node!(
    /// A free-text line: an optional leading `_`, then a mix of TEXT tokens
    /// and `[inline]` tags.
    TextLine,
    TEXT_LINE
);

/// A piece of a text line: a run of literal text, or an inline tag.
#[derive(Debug, Clone)]
pub enum TextSegment {
    /// A run of literal text (raw token, escapes unresolved).
    Text(SyntaxToken),
    /// An inline `[tag]`.
    Tag(InlineTag),
}

impl TextLine {
    /// Whether the line begins with the whitespace-preserving `_` marker.
    pub fn preserves_whitespace(&self) -> bool {
        has_token(&self.syntax, SyntaxKind::UNDERSCORE)
    }

    /// The line's segments in order: TEXT tokens and inline tags. The
    /// underscore marker and terminating newline are not segments.
    pub fn segments(&self) -> Vec<TextSegment> {
        self.syntax
            .children_with_tokens()
            .filter_map(|el| match el {
                crate::red::SyntaxElement::Token(t) if t.kind() == SyntaxKind::TEXT => {
                    Some(TextSegment::Text(t))
                }
                crate::red::SyntaxElement::Node(n) => InlineTag::cast(n).map(TextSegment::Tag),
                _ => None,
            })
            .collect()
    }

    /// The cooked text of the line: the concatenation of its TEXT tokens with
    /// `\` escapes resolved. Inline tags contribute nothing.
    pub fn cooked_text(&self) -> String {
        let mut out = String::new();
        for seg in self.segments() {
            if let TextSegment::Text(t) = seg {
                out.push_str(&resolve_escapes(t.text()));
            }
        }
        out
    }
}

// ======================================================================
// LabelLine
// ======================================================================

ast_node!(
    /// A `*name|value` label line.
    LabelLine,
    LABEL_LINE
);

impl LabelLine {
    /// The raw TEXT token inside the LABEL_NAME node, if present.
    pub fn name_token(&self) -> Option<SyntaxToken> {
        let inner = self.syntax.children().find(|n| n.kind() == SyntaxKind::LABEL_NAME)?;
        child_token(&inner, SyntaxKind::TEXT)
    }

    /// The label name verbatim (leading/trailing whitespace preserved).
    pub fn raw_name(&self) -> Option<String> {
        self.name_token().map(|t| t.text().to_string())
    }

    /// The label name, trimmed on both ends (the engine trims label segments).
    pub fn name(&self) -> Option<String> {
        self.raw_name().map(|s| s.trim().to_string())
    }

    /// The label value verbatim (the whole remainder after the first `|`),
    /// or `None` when there is no value segment.
    pub fn raw_value(&self) -> Option<String> {
        wrapped_text(&self.syntax, SyntaxKind::LABEL_VALUE)
    }

    /// The cooked label value: trimmed, and (when
    /// `label_value_first_segment_only`) truncated at the first `|`.
    ///
    /// Returns `None` when there is no LABEL_VALUE at all (no `|`, or a `|`
    /// with an empty following segment — matching the engine, which drops
    /// empty segments). An otherwise-empty value stays `Some("")`.
    pub fn value(&self, opts: &InterpretOptions) -> Option<String> {
        let raw = self.raw_value()?;
        let seg = if opts.label_value_first_segment_only {
            raw.split('|').next().unwrap_or("")
        } else {
            &raw
        };
        Some(seg.trim().to_string())
    }
}

// ======================================================================
// CharaLine
// ======================================================================

ast_node!(
    /// A `#name:face` character line.
    CharaLine,
    CHARA_LINE
);

impl CharaLine {
    /// Whether the line has a `:` separating name and face.
    fn has_colon(&self) -> bool {
        has_token(&self.syntax, SyntaxKind::COLON)
    }

    /// The character name, or `None` when absent (a bare `#`).
    ///
    /// The engine does not trim chara segments individually, but it trims
    /// the whole post-`#` remainder *before* splitting on `:`. For the
    /// name that means: leading whitespace is always removed (`# akane` →
    /// `"akane"`), trailing whitespace only when the name is the last
    /// segment (no `:` follows) — `#a : f` keeps the name as `"a "`.
    pub fn name(&self) -> Option<String> {
        let raw = wrapped_text(&self.syntax, SyntaxKind::CHARA_NAME)?;
        let raw = raw.trim_start();
        if self.has_colon() {
            Some(raw.to_string())
        } else {
            Some(raw.trim_end().to_string())
        }
    }

    /// The character face, or `None` when absent.
    ///
    /// Mirrors the engine's order of operations: the remainder is
    /// end-trimmed first (whole-line trim), *then* truncated at the first
    /// interior `:` when `chara_face_first_segment_only` — so `#a:b :c`
    /// yields `"b "` and `#a: b` yields `" b"`.
    pub fn face(&self, opts: &InterpretOptions) -> Option<String> {
        let raw = wrapped_text(&self.syntax, SyntaxKind::CHARA_FACE)?;
        let trimmed = raw.trim_end();
        let seg = if opts.chara_face_first_segment_only {
            trimmed.split(':').next().unwrap_or("")
        } else {
            trimmed
        };
        Some(seg.to_string())
    }
}

// ======================================================================
// CommentLine + BlockComment
// ======================================================================

ast_node!(
    /// A `;comment` line.
    CommentLine,
    COMMENT_LINE
);

impl CommentLine {
    /// The comment body verbatim (without the leading `;`), or `None` for an
    /// empty comment.
    pub fn text(&self) -> Option<String> {
        child_token(&self.syntax, SyntaxKind::COMMENT_TEXT).map(|t| t.text().to_string())
    }
}

ast_node!(
    /// A `/* … */` block comment spanning one or more lines.
    BlockComment,
    BLOCK_COMMENT
);

impl BlockComment {
    /// The interior comment-text lines verbatim, in order.
    pub fn text_lines(&self) -> Vec<String> {
        self.syntax
            .children_with_tokens()
            .filter_map(|e| e.into_token())
            .filter(|t| t.kind() == SyntaxKind::COMMENT_TEXT)
            .map(|t| t.text().to_string())
            .collect()
    }
}

// ======================================================================
// TagName + Tag trait
// ======================================================================

ast_node!(
    /// The tag-name node inside an inline or `@` tag.
    TagName,
    TAG_NAME
);

impl TagName {
    /// The IDENT token, if present (absent when the parser inserted a missing
    /// name).
    pub fn token(&self) -> Option<SyntaxToken> {
        child_token(&self.syntax, SyntaxKind::IDENT)
    }

    /// The tag name text (empty when the name token is missing).
    pub fn text(&self) -> String {
        self.token().map(|t| t.text().to_string()).unwrap_or_default()
    }
}

/// Shared interface of the two tag forms: `[inline]` and `@line` tags.
pub trait Tag: AstNode {
    /// The tag-name node, if present.
    fn tag_name(&self) -> Option<TagName> {
        child::<TagName>(self.syntax())
    }

    /// The tag name text (empty when missing).
    fn name(&self) -> String {
        self.tag_name().map(|n| n.text()).unwrap_or_default()
    }

    /// The tag's parameters, in order.
    fn params(&self) -> Vec<Param> {
        children::<Param>(self.syntax())
    }

    /// The first parameter with the given name.
    fn param(&self, name: &str) -> Option<Param> {
        self.params().into_iter().find(|p| p.name() == name)
    }
}

ast_node!(
    /// An `[tag param=value …]` inline tag.
    InlineTag,
    INLINE_TAG
);

ast_node!(
    /// An `@tag param=value …` whole-line tag.
    AtTagLine,
    AT_TAG_LINE
);

impl Tag for InlineTag {}
impl Tag for AtTagLine {}

/// Either tag form, for code that treats them uniformly.
#[derive(Debug, Clone)]
pub enum AnyTag {
    /// An inline `[tag]`.
    Inline(InlineTag),
    /// An `@tag` line.
    At(AtTagLine),
}

impl AnyTag {
    /// Casts a node to whichever tag form it is, or `None`.
    pub fn cast(node: SyntaxNode) -> Option<AnyTag> {
        match node.kind() {
            SyntaxKind::INLINE_TAG => InlineTag::cast(node).map(AnyTag::Inline),
            SyntaxKind::AT_TAG_LINE => AtTagLine::cast(node).map(AnyTag::At),
            _ => None,
        }
    }

    /// The underlying syntax node.
    pub fn syntax(&self) -> &SyntaxNode {
        match self {
            AnyTag::Inline(t) => t.syntax(),
            AnyTag::At(t) => t.syntax(),
        }
    }

    /// The tag-name node, if present.
    pub fn tag_name(&self) -> Option<TagName> {
        match self {
            AnyTag::Inline(t) => t.tag_name(),
            AnyTag::At(t) => t.tag_name(),
        }
    }

    /// The tag name text (empty when missing).
    pub fn name(&self) -> String {
        match self {
            AnyTag::Inline(t) => t.name(),
            AnyTag::At(t) => t.name(),
        }
    }

    /// The tag's parameters, in order.
    pub fn params(&self) -> Vec<Param> {
        match self {
            AnyTag::Inline(t) => t.params(),
            AnyTag::At(t) => t.params(),
        }
    }

    /// The first parameter with the given name.
    pub fn param(&self, name: &str) -> Option<Param> {
        match self {
            AnyTag::Inline(t) => t.param(name),
            AnyTag::At(t) => t.param(name),
        }
    }
}

// ======================================================================
// Param + ParamValue
// ======================================================================

ast_node!(
    /// One `name`, `name=`, `name=value`, or `*` parameter.
    Param,
    PARAM
);

impl Param {
    /// Whether this is the macro pass-through `*` parameter.
    pub fn is_macro_star(&self) -> bool {
        has_token(&self.syntax, SyntaxKind::STAR)
    }

    /// The parameter name (`*` for the macro pass-through parameter, empty if
    /// somehow absent).
    pub fn name(&self) -> String {
        if self.is_macro_star() {
            return "*".to_string();
        }
        child_token(&self.syntax, SyntaxKind::IDENT).map(|t| t.text().to_string()).unwrap_or_default()
    }

    /// Whether the parameter has an `=` (distinguishes `name=` / `name=value`
    /// from a bare flag `name`).
    pub fn has_eq(&self) -> bool {
        has_token(&self.syntax, SyntaxKind::EQ)
    }

    /// The value node, if the parameter has one.
    pub fn value_node(&self) -> Option<ParamValue> {
        child::<ParamValue>(&self.syntax)
    }

    /// The value token text verbatim (quotes included), if present.
    pub fn raw_value(&self) -> Option<String> {
        self.value_node().map(|v| v.raw())
    }

    /// The cooked value:
    ///
    /// - `None` for a bare flag parameter (no `=`) or the macro `*`;
    /// - `Some("")` for `name=` with no value node;
    /// - `Some(cooked)` otherwise.
    pub fn cooked_value(&self, opts: &InterpretOptions) -> Option<String> {
        if self.is_macro_star() || !self.has_eq() {
            return None;
        }
        match self.value_node() {
            Some(v) => Some(v.cooked(opts)),
            None => Some(String::new()),
        }
    }
}

ast_node!(
    /// The value of a parameter (wraps a STRING / NUMBER / TEXT / ENTITY /
    /// PARAM_REF token).
    ParamValue,
    PARAM_VALUE
);

impl ParamValue {
    /// The wrapped value token, if present.
    pub fn token(&self) -> Option<SyntaxToken> {
        self.syntax
            .children_with_tokens()
            .filter_map(|e| e.into_token())
            .find(|t| !t.is_missing())
    }

    /// The value token text verbatim (quotes and escapes included).
    pub fn raw(&self) -> String {
        self.token().map(|t| t.text().to_string()).unwrap_or_default()
    }

    /// The cooked value: quote-stripped, escape-resolved, space-handled per
    /// `opts`, with a trimmed `undefined` mapped to `""`. See [`cook_value`].
    pub fn cooked(&self, opts: &InterpretOptions) -> String {
        cook_value(&self.raw(), opts)
    }

    /// Parses this value as an expression *only* when it is an `&entity`
    /// reference: the leading `&` is stripped and the expression is anchored
    /// one byte past the token start. `None` for any other token kind.
    pub fn entity_expr(&self) -> Option<crate::expr::ExprParse> {
        let tok = self.token()?;
        if tok.kind() != SyntaxKind::ENTITY {
            return None;
        }
        let raw = tok.text();
        let interior = raw.strip_prefix('&').unwrap_or(raw);
        let anchor = tok.text_range().start() + TextSize::new(1);
        Some(crate::expr::parse_expr(interior, anchor))
    }

    /// Parses this value's raw interior as an expression (escapes *not*
    /// resolved). For a quoted STRING the delimiters are removed and the
    /// anchor is placed just after the opening quote; for an unquoted value
    /// the whole token is parsed, anchored at the token start.
    pub fn expr(&self) -> crate::expr::ExprParse {
        match self.token() {
            Some(t) => {
                let raw = t.text();
                let start = t.text_range().start();
                match raw.chars().next().filter(|c| matches!(c, '"' | '\'' | '`')) {
                    Some(q) => {
                        let qlen = q.len_utf8();
                        let mut interior = &raw[qlen..];
                        if interior.len() >= qlen && interior.ends_with(q) {
                            interior = &interior[..interior.len() - qlen];
                        }
                        let anchor = start + TextSize::new(qlen as u32);
                        crate::expr::parse_expr(interior, anchor)
                    }
                    None => crate::expr::parse_expr(raw, start),
                }
            }
            None => crate::expr::parse_expr("", self.syntax.text_range().start()),
        }
    }
}

// ======================================================================
// IScriptBlock + HtmlBlock
// ======================================================================

ast_node!(
    /// An `[iscript] … [endscript]` block.
    IScriptBlock,
    ISCRIPT_BLOCK
);

ast_node!(
    /// A `[html] … [endhtml]` block.
    HtmlBlock,
    HTML_BLOCK
);

/// First descendant tag whose name equals `name`.
fn first_tag_named(node: &SyntaxNode, name: &str) -> Option<AnyTag> {
    node.descendants().filter_map(AnyTag::cast).find(|t| t.name() == name)
}

/// Last descendant tag whose name equals `name`.
fn last_tag_named(node: &SyntaxNode, name: &str) -> Option<AnyTag> {
    node.descendants().filter_map(AnyTag::cast).filter(|t| t.name() == name).last()
}

/// Direct-child raw-block token texts of the given kind, joined with `\n`.
fn raw_block_code(node: &SyntaxNode, kind: SyntaxKind) -> String {
    node.children_with_tokens()
        .filter_map(|e| e.into_token())
        .filter(|t| t.kind() == kind)
        .map(|t| t.text().to_string())
        .collect::<Vec<_>>()
        .join("\n")
}

impl IScriptBlock {
    /// The opening `iscript` tag, if present.
    pub fn open_tag(&self) -> Option<AnyTag> {
        first_tag_named(&self.syntax, "iscript")
    }

    /// The closing `endscript` tag, or `None` when the block ended at EOF or
    /// via the loose-`endscript` quirk.
    pub fn close_tag(&self) -> Option<AnyTag> {
        last_tag_named(&self.syntax, "endscript")
    }

    /// The raw script body: the direct-child SCRIPT_TEXT tokens joined with
    /// `\n`.
    pub fn code(&self) -> String {
        raw_block_code(&self.syntax, SyntaxKind::SCRIPT_TEXT)
    }
}

impl HtmlBlock {
    /// The opening `html` tag, if present.
    pub fn open_tag(&self) -> Option<AnyTag> {
        first_tag_named(&self.syntax, "html")
    }

    /// The closing `endhtml` tag, or `None` when the block ended at EOF.
    pub fn close_tag(&self) -> Option<AnyTag> {
        last_tag_named(&self.syntax, "endhtml")
    }

    /// The raw HTML body: the direct-child HTML_TEXT tokens joined with `\n`.
    pub fn code(&self) -> String {
        raw_block_code(&self.syntax, SyntaxKind::HTML_TEXT)
    }
}

// ======================================================================
// ErrorLine
// ======================================================================

ast_node!(
    /// A syntactically invalid region preserved for losslessness.
    ErrorLine,
    ERROR
);

// ======================================================================
// Tests
// ======================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parser::parse;

    fn scenario(src: &str) -> Scenario {
        Scenario::from_parse(&parse(src))
    }

    fn only_inline_tag(src: &str) -> InlineTag {
        let scn = scenario(src);
        let line = scn.lines().next().expect("one line");
        let Line::Text(tl) = line else { panic!("expected text line") };
        tl.segments()
            .into_iter()
            .find_map(|s| match s {
                TextSegment::Tag(t) => Some(t),
                TextSegment::Text(_) => None,
            })
            .expect("an inline tag")
    }

    #[test]
    fn tag_name_and_params() {
        let tag = only_inline_tag("[bg storage=room.jpg time=1000]\n");
        assert_eq!(tag.name(), "bg");
        let opts = InterpretOptions::default();
        let params: Vec<(String, Option<String>)> =
            tag.params().iter().map(|p| (p.name(), p.cooked_value(&opts))).collect();
        assert_eq!(
            params,
            vec![
                ("storage".to_string(), Some("room.jpg".to_string())),
                ("time".to_string(), Some("1000".to_string())),
            ]
        );
        assert_eq!(tag.param("storage").unwrap().raw_value().as_deref(), Some("room.jpg"));
    }

    #[test]
    fn at_tag_strips_quotes() {
        let scn = scenario("@jump storage=\"title.ks\"\n");
        let Line::AtTag(at) = scn.lines().next().unwrap() else { panic!("at tag") };
        assert_eq!(at.name(), "jump");
        assert_eq!(
            at.param("storage").unwrap().cooked_value(&InterpretOptions::default()),
            Some("title.ks".to_string())
        );
    }

    #[test]
    fn macro_star_and_flag_params() {
        let tag = only_inline_tag("[macro_use * flag2]");
        let opts = InterpretOptions::default();
        let params: Vec<(String, bool, Option<String>)> = tag
            .params()
            .iter()
            .map(|p| (p.name(), p.is_macro_star(), p.cooked_value(&opts)))
            .collect();
        assert_eq!(
            params,
            vec![
                ("*".to_string(), true, None),
                ("flag2".to_string(), false, None),
            ]
        );
    }

    #[test]
    fn empty_and_undefined_values() {
        let opts = InterpretOptions::default();
        assert_eq!(
            only_inline_tag("[a t=]").param("t").unwrap().cooked_value(&opts),
            Some(String::new())
        );
        assert_eq!(
            only_inline_tag("[a t=\"undefined\"]").param("t").unwrap().cooked_value(&opts),
            Some(String::new())
        );
        assert_eq!(
            only_inline_tag("[a t=undefined]").param("t").unwrap().cooked_value(&opts),
            Some(String::new())
        );
    }

    #[test]
    fn flag_param_has_no_cooked_value() {
        let tag = only_inline_tag("[bg time storage=x]");
        assert_eq!(tag.param("time").unwrap().cooked_value(&InterpretOptions::default()), None);
    }

    fn cooked(src: &str, opts: &InterpretOptions) -> String {
        only_inline_tag(src).param("t").unwrap().cooked_value(opts).unwrap()
    }

    #[test]
    fn keep_space_levels() {
        let trim = InterpretOptions::default();
        let remove = InterpretOptions { keep_space: KeepSpaceLevel::RemoveAll, ..trim.clone() };
        let keep = InterpretOptions { keep_space: KeepSpaceLevel::KeepAll, ..trim.clone() };
        assert_eq!(cooked("[a t=\" x y \"]", &trim), "x y");
        assert_eq!(cooked("[a t=\" x y \"]", &remove), "xy");
        assert_eq!(cooked("[a t=\" x y \"]", &keep), " x y ");
        // Backquote is exempt from space removal at RemoveAll, but the value
        // is still trimmed by finalize (ground truth: the old scanner returns
        // the trimmed "x y", not " x y ").
        assert_eq!(cooked("[a t=` x y `]", &remove), "x y");
        assert_eq!(cooked("[a t=` x y `]", &keep), " x y ");
    }

    #[test]
    fn escape_and_quote_compensation() {
        let opts = InterpretOptions::default();
        // Escaped quote inside a quoted value survives (does not close it).
        assert_eq!(cooked(r#"[a t="a\"b"]"#, &opts), "a\"b");
        // Escaped backslash collapses to one backslash.
        assert_eq!(cooked(r#"[a t="a\\b"]"#, &opts), r"a\b");
        // Quote compensation: an unterminated quote whose line ends in `]`
        // still cooks to the interior.
        assert_eq!(cooked("[ptext t=\"abc]", &opts), "abc");
    }

    #[test]
    fn label_name_and_value() {
        let scn = scenario("*start|セーブ1\n");
        let Line::Label(l) = scn.lines().next().unwrap() else { panic!("label") };
        assert_eq!(l.name().as_deref(), Some("start"));
        assert_eq!(l.value(&InterpretOptions::default()).as_deref(), Some("セーブ1"));
    }

    #[test]
    fn label_without_value_is_none() {
        let scn = scenario("*gamestart\n");
        let Line::Label(l) = scn.lines().next().unwrap() else { panic!("label") };
        assert_eq!(l.name().as_deref(), Some("gamestart"));
        assert_eq!(l.value(&InterpretOptions::default()), None);
    }

    #[test]
    fn label_first_segment_truncation() {
        let scn = scenario("*a|b|c\n");
        let Line::Label(l) = scn.lines().next().unwrap() else { panic!("label") };
        let default = InterpretOptions::default();
        assert_eq!(l.value(&default).as_deref(), Some("b"));
        let full = InterpretOptions { label_value_first_segment_only: false, ..default };
        assert_eq!(l.value(&full).as_deref(), Some("b|c"));
    }

    #[test]
    fn label_segments_trimmed() {
        let scn = scenario("* start | 題名 \n");
        let Line::Label(l) = scn.lines().next().unwrap() else { panic!("label") };
        assert_eq!(l.name().as_deref(), Some("start"));
        assert_eq!(l.value(&InterpretOptions::default()).as_deref(), Some("題名"));
    }

    #[test]
    fn chara_name_and_face() {
        let scn = scenario("#akane:happy\n");
        let Line::Chara(c) = scn.lines().next().unwrap() else { panic!("chara") };
        assert_eq!(c.name().as_deref(), Some("akane"));
        assert_eq!(c.face(&InterpretOptions::default()).as_deref(), Some("happy"));
    }

    #[test]
    fn bare_sharp_has_no_name() {
        let scn = scenario("#\n");
        let Line::Chara(c) = scn.lines().next().unwrap() else { panic!("chara") };
        assert_eq!(c.name(), None);
        assert_eq!(c.face(&InterpretOptions::default()), None);
    }

    #[test]
    fn chara_face_truncation_and_spacing() {
        let scn = scenario("#a:b:c\n");
        let Line::Chara(c) = scn.lines().next().unwrap() else { panic!("chara") };
        let default = InterpretOptions::default();
        assert_eq!(c.face(&default).as_deref(), Some("b"));
        let full = InterpretOptions { chara_face_first_segment_only: false, ..default };
        assert_eq!(c.face(&full).as_deref(), Some("b:c"));

        // Face keeps its leading space (segments are not trimmed individually).
        let scn = scenario("#a: b\n");
        let Line::Chara(c) = scn.lines().next().unwrap() else { panic!("chara") };
        assert_eq!(c.face(&InterpretOptions::default()).as_deref(), Some(" b"));
    }

    #[test]
    fn text_segments_and_cooked() {
        let scn = scenario("こんにちは[l]世界\n");
        let Line::Text(tl) = scn.lines().next().unwrap() else { panic!("text") };
        let kinds: Vec<&str> = tl
            .segments()
            .iter()
            .map(|s| match s {
                TextSegment::Text(_) => "text",
                TextSegment::Tag(_) => "tag",
            })
            .collect();
        assert_eq!(kinds, vec!["text", "tag", "text"]);
        assert_eq!(tl.cooked_text(), "こんにちは世界");
    }

    #[test]
    fn text_escapes_resolved() {
        let scn = scenario(r"\[not a tag\]");
        let Line::Text(tl) = scn.lines().next().unwrap() else { panic!("text") };
        assert_eq!(tl.cooked_text(), "[not a tag]");
        assert!(!tl.preserves_whitespace());
    }

    #[test]
    fn underscore_preserves_whitespace() {
        let scn = scenario("_  spaced\n");
        let Line::Text(tl) = scn.lines().next().unwrap() else { panic!("text") };
        assert!(tl.preserves_whitespace());
        assert_eq!(tl.cooked_text(), "  spaced");
    }

    #[test]
    fn comment_and_block_comment() {
        let scn = scenario(";コメント\n");
        let Line::Comment(c) = scn.lines().next().unwrap() else { panic!("comment") };
        // COMMENT_TEXT is the body after the `;` (the `;` is its own token).
        assert_eq!(c.text().as_deref(), Some("コメント"));

        let scn = scenario("/*\nhidden\nlines\n*/\n");
        let Line::BlockComment(b) = scn.lines().next().unwrap() else { panic!("block") };
        assert_eq!(b.text_lines(), vec!["hidden".to_string(), "lines".to_string()]);
    }

    #[test]
    fn iscript_block_code_and_tags() {
        let scn = scenario("[iscript]\nvar a = 1;\n[endscript]\n");
        let Line::IScript(b) = scn.lines().next().unwrap() else { panic!("iscript") };
        assert_eq!(b.code(), "var a = 1;");
        assert_eq!(b.open_tag().unwrap().name(), "iscript");
        assert_eq!(b.close_tag().unwrap().name(), "endscript");
    }

    #[test]
    fn iscript_multi_line_code() {
        let scn = scenario("before[iscript]var a=1;\ncode\n[endscript]");
        let Line::IScript(b) = scn.lines().next().unwrap() else { panic!("iscript") };
        assert_eq!(b.code(), "var a=1;\ncode");
    }

    #[test]
    fn loose_endscript_quirk_line_is_outside_block() {
        let scn = scenario("[iscript]\nvar s = \"endscript\";\n[s]\n");
        let mut lines = scn.lines();
        let Line::IScript(b) = lines.next().unwrap() else { panic!("iscript") };
        // Closed by the loose quirk => no closing tag inside the block.
        assert!(b.close_tag().is_none());
        assert_eq!(b.open_tag().unwrap().name(), "iscript");
        // The quirk line surfaces as an ordinary text line in the scenario.
        assert!(matches!(lines.next(), Some(Line::Text(_))));
    }

    #[test]
    fn html_block_code() {
        let scn = scenario("[html]\n<b>x</b>\n[endhtml]\n");
        let Line::Html(b) = scn.lines().next().unwrap() else { panic!("html") };
        assert_eq!(b.code(), "<b>x</b>");
        assert_eq!(b.open_tag().unwrap().name(), "html");
        assert_eq!(b.close_tag().unwrap().name(), "endhtml");
    }

    #[test]
    fn entity_expr_and_expr() {
        let tag = only_inline_tag("[eval exp=&f.name]");
        let pv = tag.param("exp").unwrap().value_node().unwrap();
        let entity = pv.entity_expr().expect("entity expr");
        assert_eq!(entity.syntax().kind(), SyntaxKind::EXPR_ROOT);
        // A plain expr parse also yields an EXPR_ROOT.
        assert_eq!(pv.expr().syntax().kind(), SyntaxKind::EXPR_ROOT);

        // A non-entity value has no entity expr.
        let tag = only_inline_tag("[a t=1]");
        let pv = tag.param("t").unwrap().value_node().unwrap();
        assert!(pv.entity_expr().is_none());
    }

    #[test]
    fn line_enum_covers_all_kinds() {
        let src = "text\n\
                   *label\n\
                   #chara:face\n\
                   ;comment\n\
                   /*\nblock\n*/\n\
                   @wait time=100\n\
                   [iscript]\ncode\n[endscript]\n\
                   [html]\n<b/>\n[endhtml]\n";
        let scn = scenario(src);
        let variants: Vec<&str> = scn
            .lines()
            .map(|l| match l {
                Line::Text(_) => "text",
                Line::Label(_) => "label",
                Line::Chara(_) => "chara",
                Line::Comment(_) => "comment",
                Line::BlockComment(_) => "block",
                Line::AtTag(_) => "at",
                Line::IScript(_) => "iscript",
                Line::Html(_) => "html",
                Line::Error(_) => "error",
            })
            .collect();
        assert_eq!(
            variants,
            vec![
                "text", "label", "chara", "comment", "block", "at", "iscript", "html",
            ]
        );
        // No ERROR nodes for valid input.
        assert!(!scn.lines().any(|l| matches!(l, Line::Error(_))));
    }
}
