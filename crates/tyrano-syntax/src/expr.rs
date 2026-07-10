//! Pratt parser for the **embedded expression sub-language**.
//!
//! TyranoScript tag parameters embed a small JavaScript-subset expression
//! language: `exp="f.a[0]+1"`, `cond="f.x==1"`, and `&entity` references all
//! carry one such expression. This module parses **one** expression string
//! into its own lossless, error-tolerant green tree rooted at
//! [`SyntaxKind::EXPR_ROOT`]. It never fails and never panics: absent operands
//! become missing tokens wrapped in `NAME_REF`, unmatched brackets become
//! missing closers, and stray input is wrapped in `ERROR` nodes — every
//! problem surfaced as a structured [`Diagnostic`] kept outside the tree.
//! Concatenating the tree reproduces the input byte-for-byte.
//!
//! # Binding-power (Pratt) model
//!
//! Operator precedence is expressed with *binding powers* rather than a
//! grammar cascade. Every infix operator has a left and a right binding
//! power; the *precedence climbing* loop keeps folding operators whose left
//! binding power is at least the caller's `min_bp` into the growing
//! left-hand side, recursing on the right with the operator's right binding
//! power. Larger numbers bind tighter. Associativity falls out of the
//! left/right split:
//!
//! - **left-associative** operators use `bp_left < bp_right` (e.g. `+` is
//!   `15,16`): after folding `a + b`, the next `+` at power 15 is still `>=`
//!   the outer `min_bp` of 0 but the recursion on `b` used `min_bp = 16`, so
//!   a following `+` does *not* attach to `b` — it re-folds around `a + b`.
//! - **right-associative** operators use `bp_left > bp_right` (e.g. `=` is
//!   `4,3`): the recursion on the right uses the *lower* power 3, so a second
//!   `=` at power 4 attaches to the right operand, nesting rightward.
//!
//! Prefix operators (`!`, unary `-`/`+`, `typeof`) parse their operand at
//! [`PREFIX_BP`]; postfix operators — call `(`, index `[`, member `.` — bind
//! at [`POSTFIX_BP`], tighter than every infix and prefix operator, so
//! `-x.y` parses as `-(x.y)` and `f.a.b` chains left-to-right.
//!
//! The whole precedence table is the const [`INFIX_OPS`]; see it for the
//! exact numbers.

use std::sync::Arc;

use crate::diagnostics::{DiagCode, Diagnostic, SecondaryKind};
use crate::green::{Checkpoint, GreenBuilder, GreenNode, GreenTrivia};
use crate::kind::SyntaxKind;
use crate::red::SyntaxNode;
use crate::text::{TextRange, TextSize};

// ======================================================================
// Operator table
// ======================================================================

/// One infix operator and its left/right binding powers. Larger numbers bind
/// tighter; `bp_left < bp_right` is left-associative, `bp_left > bp_right` is
/// right-associative. See the [module docs](self) for the model.
struct InfixOp {
    /// The operator token kind.
    kind: SyntaxKind,
    /// Binding power on the operator's left; compared against `min_bp` to
    /// decide whether the operator folds into the current left-hand side.
    bp_left: u8,
    /// Binding power passed to the recursive parse of the right operand.
    bp_right: u8,
}

/// The complete infix precedence table, loosest to tightest.
///
/// `QUESTION` sits here for its binding powers even though it builds a
/// [`SyntaxKind::TERNARY_EXPR`] (consuming `? then : else`) rather than a
/// plain `BIN_EXPR`; every other entry builds a `BIN_EXPR`. Postfix operators
/// (`(`, `[`, `.`) are *not* in this table — they are handled separately at
/// [`POSTFIX_BP`].
const INFIX_OPS: &[InfixOp] = &[
    // COMMA — sequencing, left-assoc.
    InfixOp { kind: SyntaxKind::COMMA, bp_left: 1, bp_right: 2 },
    // Assignment — right-assoc.
    InfixOp { kind: SyntaxKind::EQ, bp_left: 4, bp_right: 3 },
    InfixOp { kind: SyntaxKind::PLUS_EQ, bp_left: 4, bp_right: 3 },
    InfixOp { kind: SyntaxKind::MINUS_EQ, bp_left: 4, bp_right: 3 },
    InfixOp { kind: SyntaxKind::STAR_EQ, bp_left: 4, bp_right: 3 },
    InfixOp { kind: SyntaxKind::SLASH_EQ, bp_left: 4, bp_right: 3 },
    InfixOp { kind: SyntaxKind::PERCENT_EQ, bp_left: 4, bp_right: 3 },
    // Ternary `?:` — right-assoc (handled specially, builds TERNARY_EXPR).
    InfixOp { kind: SyntaxKind::QUESTION, bp_left: 6, bp_right: 5 },
    // Logical OR / AND.
    InfixOp { kind: SyntaxKind::PIPE2, bp_left: 7, bp_right: 8 },
    InfixOp { kind: SyntaxKind::AMP2, bp_left: 9, bp_right: 10 },
    // Equality.
    InfixOp { kind: SyntaxKind::EQ2, bp_left: 11, bp_right: 12 },
    InfixOp { kind: SyntaxKind::EQ3, bp_left: 11, bp_right: 12 },
    InfixOp { kind: SyntaxKind::NEQ, bp_left: 11, bp_right: 12 },
    InfixOp { kind: SyntaxKind::NEQ2, bp_left: 11, bp_right: 12 },
    // Relational.
    InfixOp { kind: SyntaxKind::LT, bp_left: 13, bp_right: 14 },
    InfixOp { kind: SyntaxKind::GT, bp_left: 13, bp_right: 14 },
    InfixOp { kind: SyntaxKind::LT_EQ, bp_left: 13, bp_right: 14 },
    InfixOp { kind: SyntaxKind::GT_EQ, bp_left: 13, bp_right: 14 },
    // Additive.
    InfixOp { kind: SyntaxKind::PLUS, bp_left: 15, bp_right: 16 },
    InfixOp { kind: SyntaxKind::MINUS, bp_left: 15, bp_right: 16 },
    // Multiplicative.
    InfixOp { kind: SyntaxKind::STAR, bp_left: 17, bp_right: 18 },
    InfixOp { kind: SyntaxKind::SLASH, bp_left: 17, bp_right: 18 },
    InfixOp { kind: SyntaxKind::PERCENT, bp_left: 17, bp_right: 18 },
];

/// Binding power at which a prefix operator (`!`, unary `-`/`+`, `typeof`)
/// parses its operand. Tighter than every infix operator, looser than
/// postfix, so `-a * b` is `(-a) * b` but `-x.y` is `-(x.y)`.
const PREFIX_BP: u8 = 21;

/// Binding power of the postfix operators: call `(`, index `[`, member `.`.
/// Tightest of all, so member/call/index chains bind before any prefix or
/// infix operator.
const POSTFIX_BP: u8 = 25;

/// Maximum expression-nesting depth before graceful recovery kicks in. Guards
/// against a stack overflow on pathological input like `((((…))))`; the chain
/// `1+1+1+…` stays flat (folded iteratively) and never approaches this.
const MAX_EXPR_DEPTH: u32 = 128;

/// Looks up the infix binding powers for `kind`, or `None` if it is not an
/// infix operator.
fn infix_op(kind: SyntaxKind) -> Option<&'static InfixOp> {
    INFIX_OPS.iter().find(|o| o.kind == kind)
}

// ======================================================================
// Public API
// ======================================================================

/// The result of parsing one embedded expression.
///
/// Holds the lossless `EXPR_ROOT` green tree, the diagnostics (with ranges
/// absolutized against the anchor), and the anchor itself. Cloning is cheap.
#[derive(Debug, Clone)]
pub struct ExprParse {
    green: GreenNode,
    diagnostics: Arc<[Diagnostic]>,
    anchor: TextSize,
}

impl ExprParse {
    /// The root cursor. Offsets in this tree are **relative** to the start of
    /// the expression text (offset 0 is the first byte of the input string).
    pub fn syntax(&self) -> SyntaxNode {
        SyntaxNode::new_root(self.green.clone())
    }

    /// The underlying green tree.
    pub fn green(&self) -> &GreenNode {
        &self.green
    }

    /// Structured diagnostics, ordered by primary span start. Their ranges are
    /// **absolute**: the anchor has already been added to every primary and
    /// secondary span.
    pub fn diagnostics(&self) -> &[Diagnostic] {
        &self.diagnostics
    }

    /// The absolute byte offset of the expression text inside the enclosing
    /// `.ks` source that was passed to [`parse_expr`].
    pub fn anchor(&self) -> TextSize {
        self.anchor
    }

    /// Reconstructs the expression source from the tree; byte-identical to the
    /// input (round-trip invariant).
    pub fn to_source(&self) -> String {
        self.green.to_source()
    }
}

/// Parses `text` as one JavaScript-subset expression.
///
/// `anchor` is the absolute byte offset of `text` inside the enclosing `.ks`
/// source; it is used only to absolutize diagnostic ranges (the tree itself is
/// anchored at 0). Never fails.
pub fn parse_expr(text: &str, anchor: TextSize) -> ExprParse {
    let (raw, lex_diags) = lex(text);
    let toks = attach_trivia(text, &raw);

    let mut parser = ExprParser {
        src: text,
        toks,
        pos: 0,
        builder: GreenBuilder::new(),
        diagnostics: lex_diags,
        depth: 0,
    };
    parser.parse_root();
    let green = parser.builder.finish();
    debug_assert_eq!(
        green.full_len().to_usize(),
        text.len(),
        "expression tree must cover the whole input"
    );

    let mut diagnostics = parser.diagnostics;
    diagnostics.sort_by_key(|d| (d.primary.start(), d.primary.end()));
    let diagnostics: Vec<Diagnostic> =
        diagnostics.into_iter().map(|d| shift_diagnostic(d, anchor)).collect();

    ExprParse { green, diagnostics: diagnostics.into(), anchor }
}

/// Shifts a diagnostic's primary and secondary ranges right by `anchor`,
/// turning expression-relative offsets into absolute source offsets.
fn shift_diagnostic(mut d: Diagnostic, anchor: TextSize) -> Diagnostic {
    d.primary = d.primary + anchor;
    for (range, _) in d.secondary.iter_mut() {
        *range = *range + anchor;
    }
    d
}

// ======================================================================
// Sub-lexer
// ======================================================================

/// A raw lexer token over the expression text: a kind and a byte span.
/// `WHITESPACE` tokens are trivia and get folded onto significant tokens by
/// [`attach_trivia`].
struct RawTok {
    kind: SyntaxKind,
    start: usize,
    end: usize,
}

/// A relative `[start, end)` range as a [`TextRange`].
fn rel(start: usize, end: usize) -> TextRange {
    TextRange::new(TextSize::new(start as u32), TextSize::new(end as u32))
}

/// Whether `c` may start an identifier: ASCII letter, `_`, `$`, or any
/// non-ASCII alphanumeric (so unicode identifiers like `変数` are one token).
fn is_ident_start(c: char) -> bool {
    c == '_' || c == '$' || c.is_ascii_alphabetic() || (!c.is_ascii() && c.is_alphanumeric())
}

/// Whether `c` may continue an identifier (adds ASCII digits to the start
/// set).
fn is_ident_continue(c: char) -> bool {
    c == '_' || c == '$' || c.is_ascii_alphanumeric() || (!c.is_ascii() && c.is_alphanumeric())
}

/// Multi- and single-char operators, **longest match first** so `===` wins
/// over `==` over `=`. `:` lexes to `COLON` (reused from the chara-line
/// vocabulary as the ternary separator).
const OPERATORS: &[(&str, SyntaxKind)] = &[
    ("===", SyntaxKind::EQ3),
    ("!==", SyntaxKind::NEQ2),
    ("==", SyntaxKind::EQ2),
    ("!=", SyntaxKind::NEQ),
    ("<=", SyntaxKind::LT_EQ),
    (">=", SyntaxKind::GT_EQ),
    ("&&", SyntaxKind::AMP2),
    ("||", SyntaxKind::PIPE2),
    ("+=", SyntaxKind::PLUS_EQ),
    ("-=", SyntaxKind::MINUS_EQ),
    ("*=", SyntaxKind::STAR_EQ),
    ("/=", SyntaxKind::SLASH_EQ),
    ("%=", SyntaxKind::PERCENT_EQ),
    ("+", SyntaxKind::PLUS),
    ("-", SyntaxKind::MINUS),
    ("*", SyntaxKind::STAR),
    ("/", SyntaxKind::SLASH),
    ("%", SyntaxKind::PERCENT),
    ("!", SyntaxKind::BANG),
    ("<", SyntaxKind::LT),
    (">", SyntaxKind::GT),
    ("=", SyntaxKind::EQ),
    ("?", SyntaxKind::QUESTION),
    (":", SyntaxKind::COLON),
    (".", SyntaxKind::DOT),
    (",", SyntaxKind::COMMA),
    ("(", SyntaxKind::L_PAREN),
    (")", SyntaxKind::R_PAREN),
    ("[", SyntaxKind::L_BRACKET),
    ("]", SyntaxKind::R_BRACKET),
];

/// Longest-match an operator at the front of `rest`.
fn match_operator(rest: &str) -> Option<(usize, SyntaxKind)> {
    for &(pat, kind) in OPERATORS {
        if rest.starts_with(pat) {
            return Some((pat.len(), kind));
        }
    }
    None
}

/// Maps a lexed identifier to its keyword kind, or plain `IDENT`.
fn keyword_kind(word: &str) -> SyntaxKind {
    match word {
        "true" => SyntaxKind::TRUE_KW,
        "false" => SyntaxKind::FALSE_KW,
        "null" => SyntaxKind::NULL_KW,
        "undefined" => SyntaxKind::UNDEFINED_KW,
        "typeof" => SyntaxKind::TYPEOF_KW,
        _ => SyntaxKind::IDENT,
    }
}

/// Lexes the whole expression text into raw tokens plus relative-range
/// diagnostics (unterminated strings and stray error characters).
fn lex(text: &str) -> (Vec<RawTok>, Vec<Diagnostic>) {
    let bytes = text.as_bytes();
    let len = text.len();
    let mut toks: Vec<RawTok> = Vec::new();
    let mut diags: Vec<Diagnostic> = Vec::new();
    let mut pos = 0usize;

    while pos < len {
        let start = pos;
        // `pos` always sits on a char boundary, so this never panics.
        let c = text[pos..].chars().next().expect("pos < len");

        if c == ' ' || c == '\t' {
            pos += 1;
            while pos < len && matches!(bytes[pos], b' ' | b'\t') {
                pos += 1;
            }
            toks.push(RawTok { kind: SyntaxKind::WHITESPACE, start, end: pos });
        } else if c.is_ascii_digit() || (c == '.' && starts_fraction(text, pos)) {
            pos = lex_number(bytes, pos);
            toks.push(RawTok { kind: SyntaxKind::NUMBER, start, end: pos });
        } else if c == '"' || c == '\'' || c == '`' {
            let (end, terminated) = lex_string(text, pos, c);
            pos = end;
            toks.push(RawTok { kind: SyntaxKind::STRING, start, end });
            if !terminated {
                diags.push(Diagnostic::new(DiagCode::LexUnterminatedString, rel(start, end)));
            }
        } else if is_ident_start(c) {
            pos = lex_ident(text, pos);
            toks.push(RawTok { kind: keyword_kind(&text[start..pos]), start, end: pos });
        } else if let Some((op_len, kind)) = match_operator(&text[pos..]) {
            pos += op_len;
            toks.push(RawTok { kind, start, end: pos });
        } else {
            // Anything else: one error character, kept in the tree, reported.
            pos += c.len_utf8();
            toks.push(RawTok { kind: SyntaxKind::ERROR_TOKEN, start, end: pos });
            diags.push(Diagnostic::new(DiagCode::ExprExpectedToken, rel(start, pos)));
        }
    }

    (toks, diags)
}

/// True when a `.` at `pos` begins a fractional number (a digit follows), so
/// `.5` lexes as a number but a lone `.` lexes as the member operator.
fn starts_fraction(text: &str, pos: usize) -> bool {
    text[pos..].chars().nth(1).is_some_and(|c| c.is_ascii_digit())
}

/// Consumes a numeric literal starting at `start`: optional integer digits, an
/// optional single `.` fraction, and an optional `e`/`E` exponent with sign.
/// A leading `.5` (no integer part) is handled by the caller's dispatch.
fn lex_number(bytes: &[u8], start: usize) -> usize {
    let len = bytes.len();
    let mut i = start;
    while i < len && bytes[i].is_ascii_digit() {
        i += 1;
    }
    if i < len && bytes[i] == b'.' {
        i += 1;
        while i < len && bytes[i].is_ascii_digit() {
            i += 1;
        }
    }
    if i < len && (bytes[i] == b'e' || bytes[i] == b'E') {
        // Only commit to the exponent if it is well-formed; otherwise the
        // `e`/`E` belongs to a following identifier, not this number.
        let mut j = i + 1;
        if j < len && (bytes[j] == b'+' || bytes[j] == b'-') {
            j += 1;
        }
        if j < len && bytes[j].is_ascii_digit() {
            j += 1;
            while j < len && bytes[j].is_ascii_digit() {
                j += 1;
            }
            i = j;
        }
    }
    i
}

/// Consumes a quoted string starting at the opening `quote` at `start`. The
/// delimiters are kept in the token text; `\` escapes the next char. Returns
/// the end offset and whether a closing quote was found (an unterminated
/// string runs to end of text).
fn lex_string(text: &str, start: usize, quote: char) -> (usize, bool) {
    let len = text.len();
    let mut pos = start + quote.len_utf8();
    while pos < len {
        let c = text[pos..].chars().next().expect("pos < len");
        if c == '\\' {
            pos += 1;
            if pos < len {
                let next = text[pos..].chars().next().expect("pos < len");
                pos += next.len_utf8();
            }
        } else if c == quote {
            return (pos + c.len_utf8(), true);
        } else {
            pos += c.len_utf8();
        }
    }
    (len, false)
}

/// Consumes an identifier starting at `start`.
fn lex_ident(text: &str, start: usize) -> usize {
    let len = text.len();
    let mut pos = start;
    while pos < len {
        let c = text[pos..].chars().next().expect("pos < len");
        if is_ident_continue(c) {
            pos += c.len_utf8();
        } else {
            break;
        }
    }
    pos
}

// ======================================================================
// Trivia attachment
// ======================================================================

/// A significant token with its byte span and attached whitespace trivia
/// (each trivia piece is a `[start, end)` byte range).
struct ExprTok {
    kind: SyntaxKind,
    start: usize,
    end: usize,
    leading: Vec<(usize, usize)>,
    trailing: Vec<(usize, usize)>,
}

/// Folds the raw stream into significant tokens with attached trivia, mirroring
/// [`crate::parser`]'s policy for a single line: the very first token takes any
/// leading whitespace, and every other whitespace run trails the token before
/// it. A synthetic `EOF` token is appended; leftover trailing whitespace trails
/// the last significant token, or (for whitespace-only input) leads the `EOF`.
fn attach_trivia(text: &str, raw: &[RawTok]) -> Vec<ExprTok> {
    let mut toks: Vec<ExprTok> = Vec::new();
    let mut pending: Vec<(usize, usize)> = Vec::new();

    for r in raw {
        if r.kind == SyntaxKind::WHITESPACE {
            pending.push((r.start, r.end));
            continue;
        }
        let leading = take_leading(&mut toks, &mut pending);
        toks.push(ExprTok {
            kind: r.kind,
            start: r.start,
            end: r.end,
            leading,
            trailing: Vec::new(),
        });
    }

    let leading = take_leading(&mut toks, &mut pending);
    let len = text.len();
    toks.push(ExprTok { kind: SyntaxKind::EOF, start: len, end: len, leading, trailing: Vec::new() });
    toks
}

/// Resolves a pending whitespace run at a token boundary: it leads the next
/// token if it is the first token, otherwise trails the previous one (returning
/// an empty leading run in that case).
fn take_leading(
    toks: &mut [ExprTok],
    pending: &mut Vec<(usize, usize)>,
) -> Vec<(usize, usize)> {
    if toks.is_empty() {
        std::mem::take(pending)
    } else if pending.is_empty() {
        Vec::new()
    } else {
        let run = std::mem::take(pending);
        toks.last_mut().expect("checked non-empty").trailing = run;
        Vec::new()
    }
}

// ======================================================================
// Parser
// ======================================================================

struct ExprParser<'s> {
    src: &'s str,
    toks: Vec<ExprTok>,
    pos: usize,
    builder: GreenBuilder,
    diagnostics: Vec<Diagnostic>,
    depth: u32,
}

impl ExprParser<'_> {
    // ---- token access -------------------------------------------------

    fn kind(&self) -> SyntaxKind {
        self.toks[self.pos].kind
    }

    fn at(&self, kind: SyntaxKind) -> bool {
        self.kind() == kind
    }

    fn at_eof(&self) -> bool {
        self.at(SyntaxKind::EOF)
    }

    /// The relative byte range of the current token.
    fn current_range(&self) -> TextRange {
        let t = &self.toks[self.pos];
        rel(t.start, t.end)
    }

    /// Emits the current token (with trivia) into the tree and advances.
    fn bump(&mut self) {
        debug_assert!(!self.at_eof(), "cannot bump EOF");
        let t = self.toks[self.pos].clone_shallow();
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

    fn emit(&mut self, t: &ExprTok) {
        let leading = self.make_trivia(&t.leading);
        let trailing = self.make_trivia(&t.trailing);
        let text = &self.src[t.start..t.end];
        self.builder.token(t.kind, text, leading, trailing);
    }

    fn make_trivia(&mut self, pieces: &[(usize, usize)]) -> Vec<GreenTrivia> {
        pieces
            .iter()
            .map(|&(s, e)| self.builder.trivia(SyntaxKind::WHITESPACE, &self.src[s..e]))
            .collect()
    }

    fn missing(&mut self, kind: SyntaxKind) {
        self.builder.missing_token(kind);
    }

    fn diag(&mut self, d: Diagnostic) {
        self.diagnostics.push(d);
    }

    // ---- grammar ------------------------------------------------------

    /// Parses the whole expression under an `EXPR_ROOT`. Empty input yields a
    /// missing-operand `NAME_REF`; leftover tokens after a complete expression
    /// are wrapped in a trailing `ERROR` node. The final `EOF` token is always
    /// emitted (it may carry stray whitespace) so the round-trip holds.
    fn parse_root(&mut self) {
        self.builder.start_node(SyntaxKind::EXPR_ROOT);
        if self.at_eof() {
            self.expected_operand();
        } else {
            self.expr_bp(0);
            if !self.at_eof() {
                self.trailing_input();
            }
        }
        let eof = self.toks[self.pos].clone_shallow();
        self.emit(&eof);
        self.builder.finish_node();
    }

    /// The precedence-climbing core. Parses an expression whose operators must
    /// bind at least as tightly as `min_bp`, folding tighter operators into the
    /// left-hand side. See the [module docs](self) for the binding-power model.
    fn expr_bp(&mut self, min_bp: u8) {
        self.depth += 1;
        if self.depth > MAX_EXPR_DEPTH {
            self.overflow_recovery();
            self.depth -= 1;
            return;
        }

        let cp = self.builder.checkpoint();
        self.parse_prefix(cp);

        loop {
            let op = self.kind();

            // Postfix operators bind tightest of all.
            if POSTFIX_BP >= min_bp {
                match op {
                    SyntaxKind::L_PAREN => {
                        self.parse_call(cp);
                        continue;
                    }
                    SyntaxKind::L_BRACKET => {
                        self.parse_index(cp);
                        continue;
                    }
                    SyntaxKind::DOT => {
                        self.parse_field(cp);
                        continue;
                    }
                    _ => {}
                }
            }

            let Some(info) = infix_op(op) else { break };
            if info.bp_left < min_bp {
                break;
            }
            if op == SyntaxKind::QUESTION {
                self.parse_ternary(cp, info.bp_right);
            } else {
                self.builder.start_node_at(cp, SyntaxKind::BIN_EXPR);
                self.bump(); // operator
                self.expr_bp(info.bp_right);
                self.builder.finish_node();
            }
        }

        self.depth -= 1;
    }

    /// Parses a prefix operator chain or, failing that, a primary atom.
    ///
    /// `cp` is the checkpoint taken by the caller before this call; it is
    /// unused here (atoms and prefixes emit exactly one element) but kept for
    /// symmetry with the postfix helpers that wrap at `cp`.
    fn parse_prefix(&mut self, _cp: Checkpoint) {
        match self.kind() {
            SyntaxKind::BANG | SyntaxKind::MINUS | SyntaxKind::PLUS | SyntaxKind::TYPEOF_KW => {
                self.builder.start_node(SyntaxKind::PREFIX_EXPR);
                self.bump(); // prefix operator
                self.expr_bp(PREFIX_BP);
                self.builder.finish_node();
            }
            _ => self.parse_atom(),
        }
    }

    /// Parses a primary atom: literal, name reference, or parenthesised
    /// expression. Anything that cannot start an operand yields a
    /// missing-operand `NAME_REF`.
    fn parse_atom(&mut self) {
        match self.kind() {
            SyntaxKind::NUMBER
            | SyntaxKind::STRING
            | SyntaxKind::TRUE_KW
            | SyntaxKind::FALSE_KW
            | SyntaxKind::NULL_KW
            | SyntaxKind::UNDEFINED_KW => {
                self.builder.start_node(SyntaxKind::LITERAL);
                self.bump();
                self.builder.finish_node();
            }
            SyntaxKind::IDENT => {
                self.builder.start_node(SyntaxKind::NAME_REF);
                self.bump();
                self.builder.finish_node();
            }
            SyntaxKind::L_PAREN => {
                self.builder.start_node(SyntaxKind::PAREN_EXPR);
                let open = self.current_range();
                self.bump(); // (
                self.expr_bp(0);
                self.expect_close(SyntaxKind::R_PAREN, open);
                self.builder.finish_node();
            }
            _ => self.expected_operand(),
        }
    }

    /// Emits a `NAME_REF` containing a missing `IDENT` plus an
    /// `ExprExpectedOperand` diagnostic. Keeps the tree shape regular for
    /// consumers even where an operand was absent.
    fn expected_operand(&mut self) {
        let at = self.current_range().start();
        self.builder.start_node(SyntaxKind::NAME_REF);
        self.missing(SyntaxKind::IDENT);
        self.builder.finish_node();
        self.diag(Diagnostic::new(DiagCode::ExprExpectedOperand, TextRange::empty(at)));
    }

    /// Consumes a required closing bracket, or emits a missing one plus an
    /// `ExprUnbalancedParen` diagnostic whose secondary span points at the
    /// opener.
    fn expect_close(&mut self, close: SyntaxKind, opener: TextRange) {
        if !self.bump_if(close) {
            self.missing(close);
            self.diag(
                Diagnostic::new(
                    DiagCode::ExprUnbalancedParen,
                    TextRange::empty(self.current_range().start()),
                )
                .with_secondary(opener, SecondaryKind::OpenedHere),
            );
        }
    }

    /// Builds `TERNARY_EXPR` from the already-parsed condition at `cp`:
    /// `cond ? then : else`. The `then` branch is delimited by `:`; a missing
    /// `:` is synthesized with an `ExprExpectedToken` diagnostic. The `else`
    /// branch parses at `else_bp` (right-associative).
    fn parse_ternary(&mut self, cp: Checkpoint, else_bp: u8) {
        self.builder.start_node_at(cp, SyntaxKind::TERNARY_EXPR);
        self.bump(); // QUESTION
        self.expr_bp(0); // then-branch, terminated by the COLON
        if !self.bump_if(SyntaxKind::COLON) {
            self.missing(SyntaxKind::COLON);
            self.diag(
                Diagnostic::new(
                    DiagCode::ExprExpectedToken,
                    TextRange::empty(self.current_range().start()),
                )
                .with_expected(vec![SyntaxKind::COLON]),
            );
        }
        self.expr_bp(else_bp); // else-branch
        self.builder.finish_node();
    }

    /// Builds `CALL_EXPR` from the callee at `cp` plus a parenthesised
    /// `ARG_LIST`.
    fn parse_call(&mut self, cp: Checkpoint) {
        self.builder.start_node_at(cp, SyntaxKind::CALL_EXPR);
        self.parse_arg_list();
        self.builder.finish_node();
    }

    /// Parses `( arg , arg , … )`. Arguments parse above `COMMA` (binding power
    /// 3) so commas separate them rather than sequencing; a comma with no
    /// following argument (e.g. `f(a,)`) yields a missing-operand `NAME_REF`.
    fn parse_arg_list(&mut self) {
        self.builder.start_node(SyntaxKind::ARG_LIST);
        let open = self.current_range();
        self.bump(); // (
        if !self.at(SyntaxKind::R_PAREN) && !self.at_eof() {
            loop {
                self.expr_bp(3);
                if !self.bump_if(SyntaxKind::COMMA) {
                    break;
                }
            }
        }
        self.expect_close(SyntaxKind::R_PAREN, open);
        self.builder.finish_node();
    }

    /// Builds `INDEX_EXPR` from the base at `cp`: `base [ index ]`.
    fn parse_index(&mut self, cp: Checkpoint) {
        self.builder.start_node_at(cp, SyntaxKind::INDEX_EXPR);
        let open = self.current_range();
        self.bump(); // [
        self.expr_bp(0);
        self.expect_close(SyntaxKind::R_BRACKET, open);
        self.builder.finish_node();
    }

    /// Builds `FIELD_EXPR` from the base at `cp`: `base . name`. The field name
    /// is a bare `IDENT`; a missing name is synthesized with an
    /// `ExprExpectedToken` diagnostic.
    fn parse_field(&mut self, cp: Checkpoint) {
        self.builder.start_node_at(cp, SyntaxKind::FIELD_EXPR);
        self.bump(); // .
        if self.at(SyntaxKind::IDENT) {
            self.bump();
        } else {
            self.missing(SyntaxKind::IDENT);
            self.diag(
                Diagnostic::new(
                    DiagCode::ExprExpectedToken,
                    TextRange::empty(self.current_range().start()),
                )
                .with_expected(vec![SyntaxKind::IDENT]),
            );
        }
        self.builder.finish_node();
    }

    /// Wraps every remaining significant token in one `ERROR` node and reports
    /// `ExprTrailingInput` over their range. Reached only via stray `)`/`]`/`:`
    /// or lexer error tokens after an otherwise complete expression.
    fn trailing_input(&mut self) {
        let start = self.current_range();
        self.builder.start_node(SyntaxKind::ERROR);
        let mut end = start;
        while !self.at_eof() {
            end = self.current_range();
            self.bump();
        }
        self.builder.finish_node();
        self.diag(Diagnostic::new(DiagCode::ExprTrailingInput, start.cover(end)));
    }

    /// Graceful handling when nesting exceeds [`MAX_EXPR_DEPTH`]: rather than
    /// recurse (and risk a stack overflow), consume the rest of the input into
    /// one `ERROR` node (or emit a missing operand at `EOF`). This produces a
    /// well-formed, round-tripping tree without further recursion.
    fn overflow_recovery(&mut self) {
        if self.at_eof() {
            self.builder.start_node(SyntaxKind::NAME_REF);
            self.missing(SyntaxKind::IDENT);
            self.builder.finish_node();
            return;
        }
        let start = self.current_range();
        self.builder.start_node(SyntaxKind::ERROR);
        let mut end = start;
        while !self.at_eof() {
            end = self.current_range();
            self.bump();
        }
        self.builder.finish_node();
        self.diag(Diagnostic::new(DiagCode::ExprTrailingInput, start.cover(end)));
    }
}

impl ExprTok {
    /// A cheap copy of the token's span and trivia ranges (used to sidestep the
    /// borrow checker while emitting from `&mut self`).
    fn clone_shallow(&self) -> ExprTok {
        ExprTok {
            kind: self.kind,
            start: self.start,
            end: self.end,
            leading: self.leading.clone(),
            trailing: self.trailing.clone(),
        }
    }
}

// ======================================================================
// Tests
// ======================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use crate::green::GreenElement;

    /// Compact S-expression dump of node/token kinds, matching the parser's
    /// test format: bare token names, missing tokens with a `!` suffix.
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

    /// Parses at anchor 0 and asserts the round-trip invariant on every input.
    fn parse(src: &str) -> ExprParse {
        let p = parse_expr(src, TextSize::new(0));
        assert_eq!(p.to_source(), src, "round-trip failed for {src:?}");
        p
    }

    fn shape(src: &str) -> String {
        dump(parse(src).green())
    }

    fn has_code(p: &ExprParse, code: DiagCode) -> bool {
        p.diagnostics().iter().any(|d| d.code == code)
    }

    // -- precedence ----------------------------------------------------

    #[test]
    fn precedence_mul_binds_tighter_than_add() {
        assert_eq!(
            shape("a + b * c"),
            "(expr_root (bin_expr (name_ref ident) plus \
(bin_expr (name_ref ident) star (name_ref ident))) eof)"
        );
    }

    #[test]
    fn precedence_add_after_mul() {
        assert_eq!(
            shape("a * b + c"),
            "(expr_root (bin_expr (bin_expr (name_ref ident) star (name_ref ident)) \
plus (name_ref ident)) eof)"
        );
    }

    #[test]
    fn additive_is_left_associative() {
        assert_eq!(
            shape("a + b - c"),
            "(expr_root (bin_expr (bin_expr (name_ref ident) plus (name_ref ident)) \
minus (name_ref ident)) eof)"
        );
    }

    // -- assignment ----------------------------------------------------

    #[test]
    fn assignment_is_right_associative() {
        assert_eq!(
            shape("f.x = f.y = 1"),
            "(expr_root (bin_expr (field_expr (name_ref ident) dot ident) eq \
(bin_expr (field_expr (name_ref ident) dot ident) eq (literal number))) eof)"
        );
    }

    #[test]
    fn compound_assignment() {
        assert_eq!(
            shape("f.x += 1"),
            "(expr_root (bin_expr (field_expr (name_ref ident) dot ident) \
plus_eq (literal number)) eof)"
        );
    }

    // -- ternary -------------------------------------------------------

    #[test]
    fn ternary_basic() {
        assert_eq!(
            shape("a ? b : c"),
            "(expr_root (ternary_expr (name_ref ident) question (name_ref ident) \
colon (name_ref ident)) eof)"
        );
    }

    #[test]
    fn ternary_is_right_associative() {
        assert_eq!(
            shape("a ? b : c ? d : e"),
            "(expr_root (ternary_expr (name_ref ident) question (name_ref ident) colon \
(ternary_expr (name_ref ident) question (name_ref ident) colon (name_ref ident))) eof)"
        );
    }

    // -- comparison chain ----------------------------------------------

    #[test]
    fn comparison_and_logical_chain() {
        assert_eq!(
            shape("a == b && c != d || e === f"),
            "(expr_root (bin_expr (bin_expr (bin_expr (name_ref ident) eq2 (name_ref ident)) \
amp2 (bin_expr (name_ref ident) neq (name_ref ident))) pipe2 \
(bin_expr (name_ref ident) eq3 (name_ref ident))) eof)"
        );
    }

    // -- prefix --------------------------------------------------------

    #[test]
    fn prefix_bang() {
        assert_eq!(shape("!a"), "(expr_root (prefix_expr bang (name_ref ident)) eof)");
    }

    #[test]
    fn prefix_minus_binds_looser_than_member() {
        assert_eq!(
            shape("-x.y"),
            "(expr_root (prefix_expr minus (field_expr (name_ref ident) dot ident)) eof)"
        );
    }

    #[test]
    fn prefix_typeof() {
        assert_eq!(
            shape("typeof f.x"),
            "(expr_root (prefix_expr typeof_kw (field_expr (name_ref ident) dot ident)) eof)"
        );
    }

    #[test]
    fn double_prefix() {
        assert_eq!(
            shape("--a"),
            "(expr_root (prefix_expr minus (prefix_expr minus (name_ref ident))) eof)"
        );
    }

    // -- postfix chain -------------------------------------------------

    #[test]
    fn postfix_chain_exact() {
        assert_eq!(
            shape("f.a[0].b(1, 2).c"),
            "(expr_root (field_expr (call_expr (field_expr (index_expr \
(field_expr (name_ref ident) dot ident) l_bracket (literal number) r_bracket) dot ident) \
(arg_list l_paren (literal number) comma (literal number) r_paren)) dot ident) eof)"
        );
    }

    // -- calls ---------------------------------------------------------

    #[test]
    fn call_zero_args() {
        assert_eq!(
            shape("f()"),
            "(expr_root (call_expr (name_ref ident) (arg_list l_paren r_paren)) eof)"
        );
    }

    #[test]
    fn call_one_arg() {
        assert_eq!(
            shape("f(a)"),
            "(expr_root (call_expr (name_ref ident) (arg_list l_paren (name_ref ident) r_paren)) eof)"
        );
    }

    #[test]
    fn call_trailing_comma_is_missing_operand() {
        let p = parse("f(a,)");
        assert_eq!(
            dump(p.green()),
            "(expr_root (call_expr (name_ref ident) (arg_list l_paren (name_ref ident) comma \
(name_ref ident!) r_paren)) eof)"
        );
        assert!(has_code(&p, DiagCode::ExprExpectedOperand));
    }

    // -- parens --------------------------------------------------------

    #[test]
    fn parens_beat_precedence() {
        assert_eq!(
            shape("(a + b) * c"),
            "(expr_root (bin_expr (paren_expr l_paren \
(bin_expr (name_ref ident) plus (name_ref ident)) r_paren) star (name_ref ident)) eof)"
        );
    }

    // -- literals ------------------------------------------------------

    #[test]
    fn number_literals() {
        for src in [".5", "1e3", "2.5E-1", "42", "3.14", "10", "0"] {
            assert_eq!(shape(src), "(expr_root (literal number) eof)", "for {src:?}");
        }
    }

    #[test]
    fn string_literals_all_quotes_with_escapes() {
        for src in ["\"hi\"", "'hi'", "`hi`", "\"a\\\"b\"", "'a\\'b'", "`x\\`y`"] {
            assert_eq!(shape(src), "(expr_root (literal string) eof)", "for {src:?}");
        }
    }

    #[test]
    fn keyword_literals() {
        for (src, kw) in [
            ("true", "true_kw"),
            ("false", "false_kw"),
            ("null", "null_kw"),
            ("undefined", "undefined_kw"),
        ] {
            assert_eq!(shape(src), format!("(expr_root (literal {kw}) eof)"), "for {src:?}");
        }
    }

    // -- unicode -------------------------------------------------------

    #[test]
    fn unicode_identifier() {
        assert_eq!(
            shape("変数 + 1"),
            "(expr_root (bin_expr (name_ref ident) plus (literal number)) eof)"
        );
    }

    // -- comma sequencing ----------------------------------------------

    #[test]
    fn top_level_comma_sequencing() {
        assert_eq!(
            shape("a, b"),
            "(expr_root (bin_expr (name_ref ident) comma (name_ref ident)) eof)"
        );
    }

    // -- errors --------------------------------------------------------

    #[test]
    fn missing_operand_after_operator() {
        let p = parse("a +");
        assert_eq!(
            dump(p.green()),
            "(expr_root (bin_expr (name_ref ident) plus (name_ref ident!)) eof)"
        );
        assert!(has_code(&p, DiagCode::ExprExpectedOperand));
    }

    #[test]
    fn unclosed_paren() {
        let p = parse("(a");
        assert_eq!(
            dump(p.green()),
            "(expr_root (paren_expr l_paren (name_ref ident) r_paren!) eof)"
        );
        assert!(has_code(&p, DiagCode::ExprUnbalancedParen));
        let d = p
            .diagnostics()
            .iter()
            .find(|d| d.code == DiagCode::ExprUnbalancedParen)
            .unwrap();
        assert_eq!(d.secondary, vec![(rel(0, 1), SecondaryKind::OpenedHere)]);
    }

    #[test]
    fn ternary_missing_colon() {
        let p = parse("a ? b");
        assert_eq!(
            dump(p.green()),
            "(expr_root (ternary_expr (name_ref ident) question (name_ref ident) colon! \
(name_ref ident!)) eof)"
        );
        assert!(has_code(&p, DiagCode::ExprExpectedToken));
        assert!(has_code(&p, DiagCode::ExprExpectedOperand));
        let d = p
            .diagnostics()
            .iter()
            .find(|d| d.code == DiagCode::ExprExpectedToken)
            .unwrap();
        assert_eq!(d.expected, vec![SyntaxKind::COLON]);
    }

    #[test]
    fn stray_close_paren_is_trailing_input() {
        let p = parse(")");
        assert_eq!(
            dump(p.green()),
            "(expr_root (name_ref ident!) (error r_paren) eof)"
        );
        assert!(has_code(&p, DiagCode::ExprTrailingInput));
    }

    #[test]
    fn stray_error_char() {
        let p = parse("a @ b");
        assert_eq!(
            dump(p.green()),
            "(expr_root (name_ref ident) (error error_token ident) eof)"
        );
        assert!(has_code(&p, DiagCode::ExprExpectedToken));
        assert!(has_code(&p, DiagCode::ExprTrailingInput));
    }

    #[test]
    fn empty_input() {
        let p = parse("");
        assert_eq!(dump(p.green()), "(expr_root (name_ref ident!) eof)");
        assert!(has_code(&p, DiagCode::ExprExpectedOperand));
    }

    #[test]
    fn whitespace_only_input() {
        let p = parse("   ");
        assert_eq!(dump(p.green()), "(expr_root (name_ref ident!) eof)");
        assert!(has_code(&p, DiagCode::ExprExpectedOperand));
    }

    #[test]
    fn unterminated_string() {
        let p = parse("\"abc");
        assert_eq!(dump(p.green()), "(expr_root (literal string) eof)");
        assert!(has_code(&p, DiagCode::LexUnterminatedString));
    }

    // -- trivia --------------------------------------------------------

    #[test]
    fn whitespace_trivia_round_trips() {
        let p = parse(" a + b ");
        assert_eq!(
            dump(p.green()),
            "(expr_root (bin_expr (name_ref ident) plus (name_ref ident)) eof)"
        );
        assert_eq!(p.to_source(), " a + b ");
    }

    #[test]
    fn syntax_offsets_are_relative() {
        let p = parse_expr("a + b", TextSize::new(100));
        assert_eq!(p.syntax().text_range().start(), TextSize::new(0));
        assert_eq!(p.syntax().text_range().end(), TextSize::new(5));
    }

    // -- diagnostics anchoring -----------------------------------------

    #[test]
    fn diagnostics_are_absolutized() {
        let p = parse_expr("a +", TextSize::new(100));
        assert_eq!(p.anchor(), TextSize::new(100));
        let d = p
            .diagnostics()
            .iter()
            .find(|d| d.code == DiagCode::ExprExpectedOperand)
            .unwrap();
        // Missing operand sits at relative offset 3 (end of "a +"), so at
        // anchor 100 it is absolute 103.
        assert_eq!(d.primary.start(), TextSize::new(103));
    }

    #[test]
    fn unbalanced_secondary_is_absolutized() {
        let p = parse_expr("(a", TextSize::new(50));
        let d = p
            .diagnostics()
            .iter()
            .find(|d| d.code == DiagCode::ExprUnbalancedParen)
            .unwrap();
        // The opener `(` at relative 0..1 becomes absolute 50..51.
        assert_eq!(d.secondary, vec![(rel(50, 51), SecondaryKind::OpenedHere)]);
    }

    // -- robustness ----------------------------------------------------

    #[test]
    fn deeply_nested_parens_do_not_overflow() {
        let depth = 500;
        let src = format!("{}1{}", "(".repeat(depth), ")".repeat(depth));
        let p = parse_expr(&src, TextSize::new(0));
        // No panic; still lossless.
        assert_eq!(p.to_source(), src);
    }

    #[test]
    fn long_flat_chain_does_not_overflow() {
        // A long left-associative chain is folded iteratively, so it stays
        // shallow and never triggers the depth guard.
        let terms = 2000;
        let src = std::iter::repeat_n("1", terms).collect::<Vec<_>>().join("+");
        let p = parse_expr(&src, TextSize::new(0));
        assert_eq!(p.to_source(), src);
        assert!(p.diagnostics().is_empty(), "a clean chain has no diagnostics");
    }

    #[test]
    fn no_panic_and_round_trip_on_odd_inputs() {
        for src in [
            "", "   ", "@", ")", "]", ":", "(", "[", ".", "?", "a?", "a?b", "a:b", "f(", "f(a",
            "a[", "a.", "1..2", "1e", ".e5", "&&", "===", "a=b=c", "!!!x", "((((", "))))",
            "typeof", "-", "+", "\"", "'", "`", "'\\", "a , , b", "f(,)", "x[]", "1 2 3",
            "変数.フィールド", "\t\t", "a\nb",
        ] {
            let p = parse_expr(src, TextSize::new(7));
            assert_eq!(p.to_source(), src, "round-trip failed for {src:?}");
        }
    }
}
