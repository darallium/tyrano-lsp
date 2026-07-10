//! Error-tolerant recursive-descent parser producing the lossless CST.
//!
//! The parser walks the *significant* token stream (trivia is attached to
//! tokens during a preprocessing pass) and emits green-tree build calls.
//! It never fails: unexpected input becomes `ERROR` nodes with skipped
//! tokens, absent-but-required tokens become missing tokens (empty text,
//! flagged), and every problem is reported as a structured [`Diagnostic`]
//! kept outside the tree. `parse` always returns a tree whose
//! `to_source()` equals the input byte-for-byte.
//!
//! Line structure: every line-level node owns its terminating NEWLINE
//! token; blank lines are bare NEWLINE tokens directly under `SCENARIO`;
//! the last child of `SCENARIO` is always an EOF token which may carry
//! stray leading trivia (e.g. a whitespace-only file). Block constructs
//! (`ISCRIPT_BLOCK`, `HTML_BLOCK`, `BLOCK_COMMENT`) span multiple physical
//! lines and own their interior tokens.

use crate::diagnostics::{DiagCode, Diagnostic};
use crate::green::{Checkpoint, GreenBuilder, GreenNode, GreenTrivia};
use crate::kind::SyntaxKind;
use crate::lexer::{self, LexMode, LexOptions, LexOutput};
use crate::red::SyntaxNode;
use crate::text::{SourceText, TextEdit, TextRange};
use std::sync::Arc;

/// Options that affect the shape of the tree (interpretation options live
/// in [`crate::ast::InterpretOptions`]).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParseOptions {
    /// See [`LexOptions::loose_endscript_termination`].
    pub loose_endscript_termination: bool,
}

impl Default for ParseOptions {
    fn default() -> Self {
        ParseOptions { loose_endscript_termination: true }
    }
}

impl ParseOptions {
    fn lex_options(&self) -> LexOptions {
        LexOptions { loose_endscript_termination: self.loose_endscript_termination }
    }
}

/// The result of parsing: a green tree, diagnostics, and enough context to
/// reparse incrementally. Cloning is cheap.
#[derive(Debug, Clone)]
pub struct Parse {
    green: GreenNode,
    diagnostics: Arc<[Diagnostic]>,
    options: ParseOptions,
    source: SourceText,
    line_modes: Arc<[LexMode]>,
}

impl Parse {
    /// The root of the red (cursor) tree.
    pub fn syntax(&self) -> SyntaxNode {
        SyntaxNode::new_root(self.green.clone())
    }

    pub fn green(&self) -> &GreenNode {
        &self.green
    }

    /// Structured diagnostics, ordered by primary span start.
    pub fn diagnostics(&self) -> &[Diagnostic] {
        &self.diagnostics
    }

    pub fn options(&self) -> &ParseOptions {
        &self.options
    }

    /// The parsed source buffer (shared, with a lazy line index).
    pub fn source(&self) -> &SourceText {
        &self.source
    }

    /// Reconstructs the source from the tree. Guaranteed byte-identical to
    /// the parsed input (round-trip invariant).
    pub fn to_source(&self) -> String {
        self.green.to_source()
    }

    /// The lexer mode at the start of each physical line (useful for
    /// tooling, e.g. highlighting embedded script regions).
    pub fn line_modes(&self) -> &[LexMode] {
        &self.line_modes
    }

    /// Reparses after an edit. The result is always structurally equal to
    /// `parse_with_options(new_source, self.options())`; edits are a reuse
    /// hint, never a semantic input.
    pub fn reparse(&self, new_source: &str, edits: &[TextEdit]) -> Parse {
        crate::incremental::reparse(self, new_source, edits)
    }

    /// The typed AST view of the root (a projection — the CST stays the
    /// single source of truth).
    pub fn ast(&self) -> crate::ast::Scenario {
        crate::ast::Scenario::from_parse(self)
    }
}

/// Parses with default options.
pub fn parse(source: &str) -> Parse {
    parse_with_options(source, &ParseOptions::default())
}

/// Parses with explicit options; never fails.
pub fn parse_with_options(source: &str, options: &ParseOptions) -> Parse {
    let lexed = lexer::lex(source, &options.lex_options());
    parse_lexed(source, lexed, options)
}

pub(crate) fn parse_lexed(source: &str, lexed: LexOutput, options: &ParseOptions) -> Parse {
    parse_lexed_with_reuse(source, lexed, options, None)
}

pub(crate) fn parse_lexed_with_reuse(
    source: &str,
    lexed: LexOutput,
    options: &ParseOptions,
    reuse: Option<crate::incremental::ReuseMap>,
) -> Parse {
    let line_modes: Arc<[LexMode]> = lexed.line_modes.as_slice().into();
    let mut diagnostics = lexed.diagnostics.clone();
    let toks = attach_trivia(source, &lexed);

    let mut parser = Parser {
        src: source,
        toks,
        pos: 0,
        builder: GreenBuilder::new(),
        depth: 0,
        reuse,
    };
    parser.parse_scenario();
    let green = parser.builder.finish();
    debug_assert_eq!(
        green.full_len().to_usize(),
        source.len(),
        "tree must cover the whole input"
    );

    // Parser-level diagnostics are derived from the finished tree rather
    // than emitted while parsing: this keeps them deterministic under
    // incremental reuse (a spliced subtree yields exactly the same
    // diagnostics as a freshly parsed one).
    diagnostics.extend(derive_tree_diagnostics(&SyntaxNode::new_root(green.clone())));
    diagnostics.sort_by_key(|d| (d.primary.start(), d.primary.end()));

    Parse {
        green,
        diagnostics: diagnostics.into(),
        options: options.clone(),
        source: SourceText::new(source),
        line_modes,
    }
}

/// Diagnostics that are a pure function of the tree: `ERROR` nodes and
/// missing tag names. Lexical diagnostics (unterminated constructs, quote
/// compensation, …) come from the lexer instead.
fn derive_tree_diagnostics(root: &SyntaxNode) -> Vec<Diagnostic> {
    let mut out = Vec::new();
    for node in root.descendants() {
        match node.kind() {
            SyntaxKind::ERROR => {
                out.push(Diagnostic::new(DiagCode::ParseUnexpectedToken, node.trimmed_range()));
            }
            SyntaxKind::TAG_NAME => {
                let missing = node
                    .children_with_tokens()
                    .next()
                    .and_then(|el| el.into_token())
                    .is_some_and(|t| t.is_missing());
                if missing {
                    out.push(Diagnostic::new(
                        DiagCode::ParseExpectedTagName,
                        TextRange::empty(node.text_range().start()),
                    ));
                }
            }
            _ => {}
        }
    }
    out
}

// ---------------------------------------------------------------------
// Trivia attachment
// ---------------------------------------------------------------------

/// One trivia piece with its absolute source range.
#[derive(Debug, Clone, Copy)]
struct TriviaPiece {
    kind: SyntaxKind,
    start: usize,
    end: usize,
}

/// A significant token with attached trivia and absolute offsets.
#[derive(Debug, Clone)]
struct SigTok {
    kind: SyntaxKind,
    start: usize,
    end: usize,
    leading: Vec<TriviaPiece>,
    trailing: Vec<TriviaPiece>,
}

/// Folds the flat lexer stream into significant tokens with attached
/// trivia. Policy: a trivia run between tokens `A` and `B` becomes
/// `B`'s *leading* trivia when `A` is a NEWLINE (or the file start) —
/// i.e. line indentation and the BOM lead the first token of the line —
/// and `A`'s *trailing* trivia otherwise (same-line whitespace trails
/// the token before it). A synthetic EOF token is appended and carries
/// any leftover trivia as leading.
fn attach_trivia(source: &str, lexed: &LexOutput) -> Vec<SigTok> {
    let mut toks: Vec<SigTok> = Vec::new();
    let mut pending: Vec<TriviaPiece> = Vec::new();
    let mut pos = 0usize;
    let mut prev_was_newline = true; // file start behaves like a line start

    for raw in &lexed.tokens {
        let start = pos;
        let end = pos + raw.len.to_usize();
        pos = end;
        if raw.kind.is_trivia() {
            pending.push(TriviaPiece { kind: raw.kind, start, end });
            continue;
        }
        let leading = if prev_was_newline || toks.is_empty() {
            std::mem::take(&mut pending)
        } else if pending.is_empty() {
            Vec::new()
        } else {
            let run = std::mem::take(&mut pending);
            toks.last_mut().expect("checked non-empty").trailing = run;
            Vec::new()
        };
        toks.push(SigTok { kind: raw.kind, start, end, leading, trailing: Vec::new() });
        prev_was_newline = raw.kind == SyntaxKind::NEWLINE;
    }

    // EOF token: carries leftover trivia (whitespace-only file, trailing
    // indentation after the last newline) as leading.
    let leading = if prev_was_newline || toks.is_empty() {
        std::mem::take(&mut pending)
    } else if pending.is_empty() {
        Vec::new()
    } else {
        let run = std::mem::take(&mut pending);
        toks.last_mut().expect("checked non-empty").trailing = run;
        Vec::new()
    };
    toks.push(SigTok {
        kind: SyntaxKind::EOF,
        start: source.len(),
        end: source.len(),
        leading,
        trailing: Vec::new(),
    });
    toks
}

// ---------------------------------------------------------------------
// Parser
// ---------------------------------------------------------------------

const MAX_ERROR_DEPTH: u32 = 64;

struct Parser<'s> {
    src: &'s str,
    toks: Vec<SigTok>,
    pos: usize,
    builder: GreenBuilder,
    depth: u32,
    /// Incremental-reuse oracle: old top-level subtrees keyed by their
    /// expected new offset (see [`crate::incremental`]).
    reuse: Option<crate::incremental::ReuseMap>,
}

impl<'s> Parser<'s> {
    // ---- token access ------------------------------------------------

    fn kind(&self) -> SyntaxKind {
        self.toks[self.pos].kind
    }

    fn nth_kind(&self, n: usize) -> SyntaxKind {
        self.toks.get(self.pos + n).map_or(SyntaxKind::EOF, |t| t.kind)
    }

    fn text(&self) -> &str {
        let t = &self.toks[self.pos];
        &self.src[t.start..t.end]
    }

    fn nth_text(&self, n: usize) -> &str {
        match self.toks.get(self.pos + n) {
            Some(t) => &self.src[t.start..t.end],
            None => "",
        }
    }

    fn at(&self, kind: SyntaxKind) -> bool {
        self.kind() == kind
    }

    fn at_eof(&self) -> bool {
        self.at(SyntaxKind::EOF)
    }

    /// Emits the current token (with its trivia) into the tree.
    fn bump(&mut self) {
        assert!(!self.at_eof(), "cannot bump EOF");
        let t = self.toks[self.pos].clone();
        self.emit(&t);
        self.pos += 1;
    }

    fn bump_if(&mut self, kind: SyntaxKind) -> bool {
        if self.at(kind) && !self.at_eof() {
            self.bump();
            true
        } else {
            false
        }
    }

    fn emit(&mut self, t: &SigTok) {
        let leading = self.make_trivia(&t.leading);
        let trailing = self.make_trivia(&t.trailing);
        let text = &self.src[t.start..t.end];
        self.builder.token(t.kind, text, leading, trailing);
    }

    fn make_trivia(&mut self, pieces: &[TriviaPiece]) -> Vec<GreenTrivia> {
        pieces
            .iter()
            .map(|p| self.builder.trivia(p.kind, &self.src[p.start..p.end]))
            .collect()
    }

    /// Consumes the line-terminating NEWLINE into the current node, if any.
    fn eat_newline(&mut self) {
        self.bump_if(SyntaxKind::NEWLINE);
    }

    fn missing(&mut self, kind: SyntaxKind) {
        self.builder.missing_token(kind);
    }

    /// Byte offset where the current token's line content begins,
    /// including its leading trivia (line indentation, BOM).
    fn line_start_offset(&self, pos: usize) -> usize {
        let t = &self.toks[pos];
        t.leading.first().map_or(t.start, |p| p.start)
    }

    // ---- grammar -------------------------------------------------------

    fn parse_scenario(&mut self) {
        self.builder.start_node(SyntaxKind::SCENARIO);
        while !self.at_eof() {
            if self.try_splice() {
                continue;
            }
            self.parse_line();
        }
        // The EOF token (possibly carrying stray leading trivia).
        let eof = self.toks[self.pos].clone();
        self.emit(&eof);
        self.builder.finish_node();
    }

    /// Incremental reuse: at a top-level position (always the start of a
    /// line lexed in Default mode), splice an old green subtree whose new
    /// byte range is textually identical to what it covered in the old
    /// source. Identical text + identical entry state ⇒ identical parse,
    /// so this cannot change the resulting tree.
    fn try_splice(&mut self) -> bool {
        let Some(reuse) = &self.reuse else { return false };
        let start = self.line_start_offset(self.pos);
        let Some(node) = reuse.reusable_at(start, self.src) else { return false };
        let end = start + node.full_len().to_usize();
        self.builder.node(node);
        while !self.at_eof() && self.line_start_offset(self.pos) < end {
            self.pos += 1;
        }
        debug_assert_eq!(
            self.line_start_offset(self.pos).min(end),
            end,
            "spliced subtree must end on a token boundary"
        );
        true
    }

    /// Parses one line-level construct. The lexer's line dispatch
    /// guarantees which kinds can start a line; anything else is handled
    /// by the defensive ERROR path.
    fn parse_line(&mut self) {
        match self.kind() {
            SyntaxKind::NEWLINE => self.bump(), // blank line
            SyntaxKind::SEMICOLON => self.parse_comment_line(),
            SyntaxKind::SLASH_STAR => self.parse_block_comment(),
            SyntaxKind::STAR => self.parse_split_line(
                SyntaxKind::LABEL_LINE,
                SyntaxKind::PIPE,
                SyntaxKind::LABEL_NAME,
                SyntaxKind::LABEL_VALUE,
            ),
            SyntaxKind::SHARP => self.parse_split_line(
                SyntaxKind::CHARA_LINE,
                SyntaxKind::COLON,
                SyntaxKind::CHARA_NAME,
                SyntaxKind::CHARA_FACE,
            ),
            SyntaxKind::AT => self.parse_at_line(),
            SyntaxKind::UNDERSCORE
            | SyntaxKind::TEXT
            | SyntaxKind::L_BRACKET
            | SyntaxKind::SCRIPT_TEXT
            | SyntaxKind::HTML_TEXT => self.parse_text_line(),
            _ => self.parse_error_line(),
        }
    }

    fn parse_comment_line(&mut self) {
        self.builder.start_node(SyntaxKind::COMMENT_LINE);
        self.bump(); // SEMICOLON
        self.bump_if(SyntaxKind::COMMENT_TEXT);
        self.eat_newline();
        self.builder.finish_node();
    }

    fn parse_block_comment(&mut self) {
        self.builder.start_node(SyntaxKind::BLOCK_COMMENT);
        self.bump(); // SLASH_STAR
        loop {
            match self.kind() {
                SyntaxKind::COMMENT_TEXT | SyntaxKind::NEWLINE => self.bump(),
                SyntaxKind::STAR_SLASH => {
                    self.bump();
                    self.eat_newline();
                    break;
                }
                // EOF (unterminated: lexer already diagnosed) or, defensively,
                // anything else.
                _ => {
                    self.missing(SyntaxKind::STAR_SLASH);
                    break;
                }
            }
        }
        self.builder.finish_node();
    }

    /// `*name|value` / `#name:face` lines share one shape.
    fn parse_split_line(
        &mut self,
        line: SyntaxKind,
        sep: SyntaxKind,
        name: SyntaxKind,
        value: SyntaxKind,
    ) {
        self.builder.start_node(line);
        self.bump(); // STAR / SHARP
        if self.at(SyntaxKind::TEXT) {
            self.builder.start_node(name);
            self.bump();
            self.builder.finish_node();
        }
        if self.at(sep) {
            self.bump();
            if self.at(SyntaxKind::TEXT) {
                self.builder.start_node(value);
                self.bump();
                self.builder.finish_node();
            }
        }
        self.eat_newline();
        self.builder.finish_node();
    }

    fn parse_at_line(&mut self) {
        let cp = self.builder.checkpoint();
        self.builder.start_node(SyntaxKind::AT_TAG_LINE);
        self.bump(); // AT
        let name = self.parse_tag_name();
        self.parse_params();
        self.eat_newline();
        self.builder.finish_node();
        self.maybe_wrap_block(cp, &name);
    }

    /// A text line: optional `_`, then TEXT segments and `[inline]` tags.
    /// A `[iscript]`/`[html]` opener retroactively promotes the whole line
    /// into the corresponding block node.
    fn parse_text_line(&mut self) {
        let cp = self.builder.checkpoint();
        let mut block_end: Option<&'static str> = None;
        self.bump_if(SyntaxKind::UNDERSCORE);
        loop {
            match self.kind() {
                SyntaxKind::TEXT | SyntaxKind::SCRIPT_TEXT | SyntaxKind::HTML_TEXT => self.bump(),
                SyntaxKind::L_BRACKET => {
                    let name = self.parse_inline_tag();
                    block_end = block_end.or(end_tag_for(&name));
                }
                _ => break,
            }
        }
        match block_end {
            None => {
                self.builder.start_node_at(cp, SyntaxKind::TEXT_LINE);
                self.eat_newline();
                self.builder.finish_node();
            }
            Some(end) => {
                let kind = block_kind_for(end);
                self.builder.start_node_at(cp, kind);
                self.eat_newline();
                self.parse_raw_block_rest(end);
                self.builder.finish_node();
            }
        }
    }

    /// After an `@iscript`-style opener line: wraps it into a block node
    /// and parses the raw interior plus the closing tag line.
    fn maybe_wrap_block(&mut self, cp: Checkpoint, tag_name: &str) {
        let Some(end) = end_tag_for(tag_name) else { return };
        let kind = block_kind_for(end);
        self.builder.start_node_at(cp, kind);
        self.parse_raw_block_rest(end);
        self.builder.finish_node();
    }

    /// The interior of an iscript/html block: raw lines and blank lines,
    /// then — when present — the closing tag line. When the block was
    /// terminated by the loose-`endscript` quirk or EOF there is no
    /// closing line inside the block (the quirk line belongs to the
    /// scenario; the lexer has already diagnosed both cases).
    fn parse_raw_block_rest(&mut self, end_tag: &str) {
        loop {
            match self.kind() {
                SyntaxKind::SCRIPT_TEXT | SyntaxKind::HTML_TEXT | SyntaxKind::NEWLINE => {
                    self.bump()
                }
                SyntaxKind::EOF => break,
                _ => {
                    if self.at_tag_line_named(end_tag) {
                        self.parse_line();
                    }
                    break;
                }
            }
        }
    }

    /// Is the current position the start of a `@name` / `[name …]` line?
    fn at_tag_line_named(&self, name: &str) -> bool {
        match self.kind() {
            SyntaxKind::AT | SyntaxKind::L_BRACKET => {
                self.nth_kind(1) == SyntaxKind::IDENT && self.nth_text(1) == name
            }
            _ => false,
        }
    }

    /// `[tag param=value …]`; returns the tag name text.
    fn parse_inline_tag(&mut self) -> String {
        self.builder.start_node(SyntaxKind::INLINE_TAG);
        self.bump(); // L_BRACKET
        let name = self.parse_tag_name();
        self.parse_params();
        if !self.bump_if(SyntaxKind::R_BRACKET) {
            // Unterminated tag: the lexer already emitted the diagnostic
            // with the opener as secondary span.
            self.missing(SyntaxKind::R_BRACKET);
        }
        self.builder.finish_node();
        name
    }

    fn parse_tag_name(&mut self) -> String {
        self.builder.start_node(SyntaxKind::TAG_NAME);
        let name = if self.at(SyntaxKind::IDENT) {
            let name = self.text().to_string();
            self.bump();
            name
        } else {
            // Diagnosed after the fact by `derive_tree_diagnostics`.
            self.missing(SyntaxKind::IDENT);
            String::new()
        };
        self.builder.finish_node();
        name
    }

    /// Zero or more parameters: `name`, `name=`, `name=value`, or the
    /// macro pass-through `*`.
    fn parse_params(&mut self) {
        loop {
            match self.kind() {
                SyntaxKind::IDENT => {
                    self.builder.start_node(SyntaxKind::PARAM);
                    self.bump();
                    if self.bump_if(SyntaxKind::EQ) && self.at_value() {
                        self.builder.start_node(SyntaxKind::PARAM_VALUE);
                        self.bump();
                        self.builder.finish_node();
                    }
                    self.builder.finish_node();
                }
                SyntaxKind::STAR => {
                    self.builder.start_node(SyntaxKind::PARAM);
                    self.bump();
                    self.builder.finish_node();
                }
                _ => break,
            }
        }
    }

    fn at_value(&self) -> bool {
        matches!(
            self.kind(),
            SyntaxKind::STRING
                | SyntaxKind::NUMBER
                | SyntaxKind::TEXT
                | SyntaxKind::ENTITY
                | SyntaxKind::PARAM_REF
        )
    }

    /// Defensive recovery: wraps unexpected tokens (up to the next
    /// NEWLINE, inclusive) in an ERROR node so the tree stays lossless.
    /// The corresponding diagnostic is derived from the tree afterwards.
    fn parse_error_line(&mut self) {
        self.depth += 1;
        self.builder.start_node(SyntaxKind::ERROR);
        while !self.at_eof() {
            let was_newline = self.at(SyntaxKind::NEWLINE);
            self.bump();
            if was_newline {
                break;
            }
        }
        self.builder.finish_node();
        assert!(self.depth < MAX_ERROR_DEPTH, "error recovery must make progress");
        self.depth -= 1;
    }
}

fn end_tag_for(open: &str) -> Option<&'static str> {
    match open {
        "iscript" => Some("endscript"),
        "html" => Some("endhtml"),
        _ => None,
    }
}

fn block_kind_for(end_tag: &str) -> SyntaxKind {
    if end_tag == "endscript" { SyntaxKind::ISCRIPT_BLOCK } else { SyntaxKind::HTML_BLOCK }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::green::GreenElement;

    /// Compact S-expression dump of node kinds (tokens as bare names,
    /// missing tokens with a `!` suffix).
    fn dump(node: &GreenNode) -> String {
        fn go(el: &GreenElement, out: &mut String) {
            match el {
                GreenElement::Node(n) => {
                    out.push('(');
                    out.push_str(n.kind().name());
                    for c in n.children() {
                        out.push(' ');
                        go(c, out);
                    }
                    out.push(')');
                }
                GreenElement::Token(t) => {
                    out.push_str(t.kind().name());
                    if t.is_missing() {
                        out.push('!');
                    }
                }
            }
        }
        let mut out = String::new();
        out.push('(');
        out.push_str(node.kind().name());
        for c in node.children() {
            out.push(' ');
            go(c, &mut out);
        }
        out.push(')');
        out
    }

    fn roundtrip(src: &str) -> Parse {
        let p = parse(src);
        assert_eq!(p.to_source(), src, "round-trip failed");
        p
    }

    #[test]
    fn empty_input() {
        let p = roundtrip("");
        assert_eq!(dump(p.green()), "(scenario eof)");
    }

    #[test]
    fn whitespace_only_file_lands_on_eof() {
        let p = roundtrip("   ");
        assert_eq!(dump(p.green()), "(scenario eof)");
        assert_eq!(p.green().full_len().to_usize(), 3);
    }

    #[test]
    fn text_line_shape() {
        let p = roundtrip("こんにちは[l]世界\n");
        assert_eq!(
            dump(p.green()),
            "(scenario (text_line text (inline_tag l_bracket (tag_name ident) r_bracket) text newline) eof)"
        );
    }

    #[test]
    fn tag_params_shape() {
        let p = roundtrip("[bg storage=room.jpg time=1000]\n");
        assert_eq!(
            dump(p.green()),
            "(scenario (text_line (inline_tag l_bracket (tag_name ident) \
(param ident eq (param_value text)) (param ident eq (param_value number)) \
r_bracket) newline) eof)"
        );
    }

    #[test]
    fn at_tag_line_shape() {
        let p = roundtrip("@jump storage=\"title.ks\"\n");
        assert_eq!(
            dump(p.green()),
            "(scenario (at_tag_line at (tag_name ident) (param ident eq (param_value string)) newline) eof)"
        );
    }

    #[test]
    fn macro_star_and_flag_param() {
        let p = roundtrip("[macro_use * flag2]");
        assert_eq!(
            dump(p.green()),
            "(scenario (text_line (inline_tag l_bracket (tag_name ident) (param star) (param ident) r_bracket)) eof)"
        );
    }

    #[test]
    fn empty_value_param() {
        let p = roundtrip("[a t=]");
        assert_eq!(
            dump(p.green()),
            "(scenario (text_line (inline_tag l_bracket (tag_name ident) (param ident eq) r_bracket)) eof)"
        );
    }

    #[test]
    fn label_and_chara_lines() {
        assert_eq!(
            dump(roundtrip("*start|セーブ1\n").green()),
            "(scenario (label_line star (label_name text) pipe (label_value text) newline) eof)"
        );
        assert_eq!(
            dump(roundtrip("*gamestart\n").green()),
            "(scenario (label_line star (label_name text) newline) eof)"
        );
        assert_eq!(
            dump(roundtrip("#akane:happy\n").green()),
            "(scenario (chara_line sharp (chara_name text) colon (chara_face text) newline) eof)"
        );
        assert_eq!(dump(roundtrip("#\n").green()), "(scenario (chara_line sharp newline) eof)");
        assert_eq!(
            dump(roundtrip("#:face\n").green()),
            "(scenario (chara_line sharp colon (chara_face text) newline) eof)"
        );
    }

    #[test]
    fn comment_lines() {
        assert_eq!(
            dump(roundtrip(";コメント\n").green()),
            "(scenario (comment_line semicolon comment_text newline) eof)"
        );
        assert_eq!(
            dump(roundtrip("/*\nhidden\n*/\n").green()),
            "(scenario (block_comment slash_star newline comment_text newline star_slash newline) eof)"
        );
    }

    #[test]
    fn unterminated_block_comment_gets_missing_closer() {
        let p = roundtrip("/*\nhidden");
        assert_eq!(
            dump(p.green()),
            "(scenario (block_comment slash_star newline comment_text star_slash!) eof)"
        );
        assert!(
            p.diagnostics()
                .iter()
                .any(|d| matches!(d.code, DiagCode::LexUnterminatedBlockComment))
        );
    }

    #[test]
    fn unterminated_inline_tag_gets_missing_bracket() {
        let p = roundtrip("[ptext text=abc");
        assert_eq!(
            dump(p.green()),
            "(scenario (text_line (inline_tag l_bracket (tag_name ident) \
(param ident eq (param_value text)) r_bracket!)) eof)"
        );
        assert!(p.diagnostics().iter().any(|d| matches!(d.code, DiagCode::ParseUnterminatedTag)));
    }

    #[test]
    fn empty_inline_tag_gets_missing_name() {
        let p = roundtrip("[]");
        assert_eq!(
            dump(p.green()),
            "(scenario (text_line (inline_tag l_bracket (tag_name ident!) r_bracket)) eof)"
        );
        assert!(p.diagnostics().iter().any(|d| matches!(d.code, DiagCode::ParseExpectedTagName)));
    }

    #[test]
    fn iscript_block_structure() {
        let p = roundtrip("[iscript]\nvar a = 1;\n[endscript]\n");
        assert_eq!(
            dump(p.green()),
            "(scenario (iscript_block (inline_tag l_bracket (tag_name ident) r_bracket) newline \
script_text newline \
(text_line (inline_tag l_bracket (tag_name ident) r_bracket) newline)) eof)"
        );
    }

    #[test]
    fn at_iscript_block_structure() {
        let p = roundtrip("@iscript\nvar a = 1;\n@endscript\n");
        assert_eq!(
            dump(p.green()),
            "(scenario (iscript_block (at_tag_line at (tag_name ident) newline) \
script_text newline \
(at_tag_line at (tag_name ident) newline)) eof)"
        );
    }

    #[test]
    fn loose_endscript_line_is_outside_block() {
        let p = roundtrip("[iscript]\nvar s = \"endscript\";\n[s]\n");
        assert_eq!(
            dump(p.green()),
            "(scenario (iscript_block (inline_tag l_bracket (tag_name ident) r_bracket) newline) \
(text_line text newline) \
(text_line (inline_tag l_bracket (tag_name ident) r_bracket) newline) eof)"
        );
    }

    #[test]
    fn unterminated_iscript_block() {
        let p = roundtrip("[iscript]\nvar a = 1;");
        assert_eq!(
            dump(p.green()),
            "(scenario (iscript_block (inline_tag l_bracket (tag_name ident) r_bracket) newline script_text) eof)"
        );
        assert!(
            p.diagnostics().iter().any(|d| matches!(d.code, DiagCode::LexUnterminatedIscript))
        );
    }

    #[test]
    fn html_block_structure() {
        let p = roundtrip("[html]\n<b>x</b>\n[endhtml]\n");
        assert_eq!(
            dump(p.green()),
            "(scenario (html_block (inline_tag l_bracket (tag_name ident) r_bracket) newline \
html_text newline \
(text_line (inline_tag l_bracket (tag_name ident) r_bracket) newline)) eof)"
        );
    }

    #[test]
    fn inline_iscript_with_rest_of_line() {
        let p = roundtrip("before[iscript]var a=1;\ncode\n[endscript]");
        assert_eq!(
            dump(p.green()),
            "(scenario (iscript_block text (inline_tag l_bracket (tag_name ident) r_bracket) \
script_text newline script_text newline \
(text_line (inline_tag l_bracket (tag_name ident) r_bracket))) eof)"
        );
    }

    #[test]
    fn blank_lines_are_scenario_children() {
        let p = roundtrip("a\n\nb\n");
        assert_eq!(
            dump(p.green()),
            "(scenario (text_line text newline) newline (text_line text newline) eof)"
        );
    }

    #[test]
    fn underscore_line() {
        let p = roundtrip("_  spaced\n");
        assert_eq!(dump(p.green()), "(scenario (text_line underscore text newline) eof)");
    }

    #[test]
    fn trivia_roundtrip_everywhere() {
        for src in [
            "\u{feff}text\n",
            "   indented\n",
            "  [a p = 1]  \n",
            "*label \n",
            "\t@wait time=100\t\n",
            "a\r\nb\r\n",
            "/*\n  inner\n  */  \n",
            "  \n\n   ",
        ] {
            roundtrip(src);
        }
    }

    #[test]
    fn diagnostics_are_sorted() {
        let p = parse("[]\n[]\n");
        let starts: Vec<_> = p.diagnostics().iter().map(|d| d.primary.start()).collect();
        let mut sorted = starts.clone();
        sorted.sort();
        assert_eq!(starts, sorted);
    }

    #[test]
    fn no_panic_and_roundtrip_on_odd_inputs() {
        for src in [
            "\\", "[", "]", "[]", "[ ]", "@", "@ ", "*", "#\n", "_", ";", "/*", "*/x",
            "[a=b=c]", "[a \"q]", "\u{feff}", "\r", "a\rb", "@iscript", "[iscript]",
            "[html]x[endhtml]", "*|", "#:", "[a t=\"unclosed", "@a t=\"unclosed\n次",
        ] {
            roundtrip(src);
        }
    }
}
