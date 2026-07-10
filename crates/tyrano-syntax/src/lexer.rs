//! Lossless lexer for TyranoScript.
//!
//! Produces a flat stream of [`RawToken`]s that covers **every byte** of the
//! input: whitespace, BOM, escape backslashes, quote characters, comment
//! bodies, and invalid text all survive as token text. Trivia attachment
//! (leading/trailing) is the tree builder's job, not the lexer's — here
//! trivia kinds ([`SyntaxKind::WHITESPACE`], [`SyntaxKind::BOM`]) are just
//! ordinary stream entries.
//!
//! The lexer is an explicit state machine and takes no feedback from the
//! parser. Cross-line state is a single [`LexMode`]; the tag-body machine
//! (the engine's `makeTag` five-state loop) is local to
//! [`Lexer::lex_tag_body`].
//!
//! Compatibility notes: the reference engine (`kag.parser.js`) *trims* every
//! line and *destroys* escapes/quotes while tokenizing. This lexer instead
//! keeps raw text and leaves the engine's interpretations (value cooking,
//! segment truncation, `undefined` coercion) to the AST view layer. The one
//! engine behaviour that changes token boundaries — quote compensation for
//! `[tag p="v]` — is reproduced here and reported as a diagnostic. The
//! `loose_endscript_termination` quirk changes block structure, so it is a
//! lexer option.

use crate::diagnostics::{DiagCode, Diagnostic, SecondaryKind};
use crate::kind::SyntaxKind;
use crate::text::{TextRange, TextSize};

/// Options that affect token/block structure (not interpretation).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LexOptions {
    /// Engine quirk: any line merely *containing* the substring
    /// `endscript` terminates an `[iscript]` block, and that line is then
    /// lexed as ordinary scenario content.
    pub loose_endscript_termination: bool,
}

impl Default for LexOptions {
    fn default() -> Self {
        LexOptions { loose_endscript_termination: true }
    }
}

/// Cross-line lexer mode, recorded per physical line for incremental reuse.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum LexMode {
    Default,
    IScript,
    Html,
    BlockComment,
}

/// One entry of the flat token stream: a kind and its byte length.
/// Offsets are implicit (tokens are contiguous from offset 0).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RawToken {
    pub kind: SyntaxKind,
    pub len: TextSize,
}

/// Result of lexing: tokens covering the whole input, lexical diagnostics,
/// and the mode at the start of each physical line (for incremental reuse).
#[derive(Debug, Clone)]
pub struct LexOutput {
    pub tokens: Vec<RawToken>,
    pub diagnostics: Vec<Diagnostic>,
    /// `line_modes[i]` is the [`LexMode`] in force at the start of physical
    /// line `i` (lines are separated by `\n`).
    pub line_modes: Vec<LexMode>,
}

/// Lexes `source` completely. The output tokens always satisfy
/// `Σ len == source.len()`.
pub fn lex(source: &str, opts: &LexOptions) -> LexOutput {
    let mut lexer = Lexer {
        src: source,
        opts: opts.clone(),
        mode: LexMode::Default,
        emitted: 0,
        tokens: Vec::new(),
        diagnostics: Vec::new(),
        line_modes: Vec::new(),
    };
    lexer.run();
    debug_assert_eq!(lexer.emitted, source.len(), "lexer must cover every byte");
    LexOutput {
        tokens: lexer.tokens,
        diagnostics: lexer.diagnostics,
        line_modes: lexer.line_modes,
    }
}

/// How a tag body ends: at a `]` (inline tags) or at end of line (`@` tags).
/// For inline tags, `compensated` records that the closing `]` was
/// reclaimed from an unterminated quoted value (engine quirk).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum TagEnd {
    Bracket { compensated: bool },
    Eol,
}

struct Lexer<'s> {
    src: &'s str,
    opts: LexOptions,
    mode: LexMode,
    /// End offset of the last emitted token; tokens are contiguous.
    emitted: usize,
    tokens: Vec<RawToken>,
    diagnostics: Vec<Diagnostic>,
    line_modes: Vec<LexMode>,
}

impl<'s> Lexer<'s> {
    fn run(&mut self) {
        let bom_len = if self.src.starts_with('\u{feff}') { '\u{feff}'.len_utf8() } else { 0 };
        if bom_len > 0 {
            self.emit(SyntaxKind::BOM, 0, bom_len);
        }

        let len = self.src.len();
        let mut pos = bom_len;
        loop {
            self.line_modes.push(self.mode);
            // Locate this physical line: content, then `\n` / `\r\n`.
            let nl = self.src[pos..].find('\n').map(|i| pos + i);
            let (content_end, newline) = match nl {
                Some(nl) => {
                    let content_end =
                        if nl > pos && self.src.as_bytes()[nl - 1] == b'\r' { nl - 1 } else { nl };
                    (content_end, Some((content_end, nl + 1)))
                }
                None => (len, None),
            };

            self.lex_line(pos, content_end);
            debug_assert_eq!(self.emitted, content_end);

            match newline {
                Some((start, end)) => {
                    self.emit(SyntaxKind::NEWLINE, start, end);
                    pos = end;
                    if pos == len {
                        break;
                    }
                }
                None => break,
            }
        }

        match self.mode {
            LexMode::IScript => self.diag(Diagnostic::new(
                DiagCode::LexUnterminatedIscript,
                TextRange::empty(size(len)),
            )),
            LexMode::Html => self.diag(Diagnostic::new(
                DiagCode::LexUnterminatedHtml,
                TextRange::empty(size(len)),
            )),
            LexMode::BlockComment => self.diag(Diagnostic::new(
                DiagCode::LexUnterminatedBlockComment,
                TextRange::empty(size(len)),
            )),
            LexMode::Default => {}
        }
    }

    /// Lexes one physical line's content (`[start, end)`, no newline).
    fn lex_line(&mut self, start: usize, end: usize) {
        match self.mode {
            LexMode::BlockComment => self.lex_block_comment_line(start, end),
            LexMode::IScript => self.lex_raw_block_line(start, end, "endscript"),
            LexMode::Html => self.lex_raw_block_line(start, end, "endhtml"),
            LexMode::Default => self.dispatch_line(start, end),
        }
    }

    /// Inside `/* … */`: only a line whose trimmed content is exactly `*/`
    /// closes the comment; every other line is opaque comment text.
    fn lex_block_comment_line(&mut self, start: usize, end: usize) {
        let content = &self.src[start..end];
        if content.trim() == "*/" {
            let marker = start + offset_of_trimmed(content);
            self.ws(start, marker);
            self.emit(SyntaxKind::STAR_SLASH, marker, marker + 2);
            self.ws(marker + 2, end);
            self.mode = LexMode::Default;
        } else if !content.is_empty() {
            self.emit(SyntaxKind::COMMENT_TEXT, start, end);
        }
    }

    /// Inside `[iscript]` / `[html]`: raw lines until the end tag.
    fn lex_raw_block_line(&mut self, start: usize, end: usize, end_tag: &str) {
        let content = &self.src[start..end];
        let is_iscript = end_tag == "endscript";

        if line_starts_with_tag(content.trim_start(), end_tag) {
            // A real end tag: leave the mode, then lex the line as an
            // ordinary tag line so the end tag itself stays in the tree.
            self.mode = LexMode::Default;
            self.dispatch_line(start, end);
            return;
        }
        if is_iscript && self.opts.loose_endscript_termination && content.contains(end_tag) {
            // Engine quirk: the block ends on ANY line containing the
            // substring, and the line is then ordinary scenario content.
            self.mode = LexMode::Default;
            self.diag(Diagnostic::new(
                DiagCode::CompatLooseEndscript,
                TextRange::new(size(start), size(end)),
            ));
            self.dispatch_line(start, end);
            return;
        }

        if !content.is_empty() {
            let kind = if is_iscript { SyntaxKind::SCRIPT_TEXT } else { SyntaxKind::HTML_TEXT };
            self.emit(kind, start, end);
        }
    }

    /// Default-mode line dispatch on the first non-whitespace character,
    /// mirroring the engine's `parseScenario`.
    fn dispatch_line(&mut self, start: usize, end: usize) {
        let content = &self.src[start..end];
        let trimmed_start = start + offset_of_trimmed(content);
        self.ws(start, trimmed_start);
        if trimmed_start == end {
            return; // blank / whitespace-only line
        }

        if content.trim() == "/*" {
            self.emit(SyntaxKind::SLASH_STAR, trimmed_start, trimmed_start + 2);
            self.ws(trimmed_start + 2, end);
            self.mode = LexMode::BlockComment;
            return;
        }

        let first = self.src[trimmed_start..].chars().next().expect("non-empty rest");
        match first {
            ';' => {
                self.emit(SyntaxKind::SEMICOLON, trimmed_start, trimmed_start + 1);
                self.emit_nonempty(SyntaxKind::COMMENT_TEXT, trimmed_start + 1, end);
            }
            '*' => {
                self.emit(SyntaxKind::STAR, trimmed_start, trimmed_start + 1);
                self.lex_split_segments(trimmed_start + 1, end, b'|', SyntaxKind::PIPE);
            }
            '#' => {
                self.emit(SyntaxKind::SHARP, trimmed_start, trimmed_start + 1);
                self.lex_split_segments(trimmed_start + 1, end, b':', SyntaxKind::COLON);
            }
            '@' => {
                self.emit(SyntaxKind::AT, trimmed_start, trimmed_start + 1);
                let name = self.lex_tag_body(trimmed_start + 1, end, TagEnd::Eol);
                self.enter_block_mode_if_needed(&name);
            }
            '_' => {
                self.emit(SyntaxKind::UNDERSCORE, trimmed_start, trimmed_start + 1);
                self.lex_text_content(trimmed_start + 1, end);
            }
            _ => self.lex_text_content(trimmed_start, end),
        }
    }

    /// `*name|value` and `#name:face` lines: the raw segment before the
    /// first separator, the separator, and the raw remainder (which may
    /// itself contain more separators — truncation is an interpretation,
    /// not a token boundary).
    fn lex_split_segments(&mut self, start: usize, end: usize, sep: u8, sep_kind: SyntaxKind) {
        match self.src.as_bytes()[start..end].iter().position(|&b| b == sep) {
            Some(i) => {
                let sep_at = start + i;
                self.emit_nonempty(SyntaxKind::TEXT, start, sep_at);
                self.emit(sep_kind, sep_at, sep_at + 1);
                self.emit_nonempty(SyntaxKind::TEXT, sep_at + 1, end);
            }
            None => self.emit_nonempty(SyntaxKind::TEXT, start, end),
        }
    }

    /// Free text, possibly mixing `[inline tags]`. Whitespace here is
    /// content, never trivia; `\` escapes stay in the token text.
    fn lex_text_content(&mut self, start: usize, end: usize) {
        let bytes = self.src.as_bytes();
        let mut run_start = start;
        let mut i = start;
        while i < end {
            match bytes[i] {
                b'\\' => {
                    // Escape: backslash and the escaped char are plain text.
                    i += 1;
                    if i < end {
                        i += next_char_len(self.src, i);
                    }
                }
                b'[' => {
                    self.emit_nonempty(SyntaxKind::TEXT, run_start, i);
                    i = self.lex_inline_tag(i, end);
                    if self.mode != LexMode::Default {
                        // `[iscript]` / `[html]`: the rest of the line is
                        // already raw block content.
                        let kind = if self.mode == LexMode::IScript {
                            SyntaxKind::SCRIPT_TEXT
                        } else {
                            SyntaxKind::HTML_TEXT
                        };
                        self.emit_nonempty(kind, i, end);
                        return;
                    }
                    run_start = i;
                }
                _ => i += next_char_len(self.src, i),
            }
        }
        self.emit_nonempty(SyntaxKind::TEXT, run_start, end);
    }

    /// Lexes one `[tag …]` starting at the `[` at `open`. Returns the
    /// offset just past the tag (past `]`, or `end` when unterminated).
    fn lex_inline_tag(&mut self, open: usize, end: usize) -> usize {
        // The engine first extracts the bracket-balanced, quote-aware body,
        // then tokenizes it; we mirror that in one pass over the extent.
        let (body_end, closed, compensated) = self.find_tag_extent(open + 1, end);

        self.emit(SyntaxKind::L_BRACKET, open, open + 1);
        let name = self.lex_tag_body(open + 1, body_end, TagEnd::Bracket { compensated });
        if closed {
            self.emit(SyntaxKind::R_BRACKET, body_end, body_end + 1);
        } else {
            self.diag(
                Diagnostic::new(DiagCode::ParseUnterminatedTag, TextRange::empty(size(body_end)))
                    .with_secondary(
                        TextRange::new(size(open), size(open + 1)),
                        SecondaryKind::OpenedHere,
                    ),
            );
        }
        self.enter_block_mode_if_needed(&name);
        if closed { body_end + 1 } else { body_end }
    }

    /// Finds the end of an inline tag body: a `]` at bracket depth zero,
    /// honouring `\` escapes and `"` `'` `` ` `` quotes (brackets inside
    /// quotes don't count). Returns `(body_end, closed, compensated)`.
    fn find_tag_extent(&self, body_start: usize, end: usize) -> (usize, bool, bool) {
        let bytes = self.src.as_bytes();
        let mut depth = 0usize;
        let mut quote: Option<u8> = None;
        let mut i = body_start;
        while i < end {
            let b = bytes[i];
            match quote {
                Some(q) => match b {
                    b'\\' => {
                        i += 1;
                        if i < end {
                            i += next_char_len(self.src, i);
                        }
                        continue;
                    }
                    _ if b == q => quote = None,
                    _ => {}
                },
                None => match b {
                    b'\\' => {
                        i += 1;
                        if i < end {
                            i += next_char_len(self.src, i);
                        }
                        continue;
                    }
                    b'"' | b'\'' | b'`' => quote = Some(b),
                    b'[' => depth += 1,
                    b']' if depth > 0 => depth -= 1,
                    b']' => return (i, true, false),
                    _ => {}
                },
            }
            i += next_char_len(self.src, i);
        }
        // Unterminated bracket: the engine's quote compensation — if a
        // quote is still open and the line's last char is `]`, that `]`
        // closes the tag after all (`[tag p="v]`).
        if quote.is_some() && end > body_start && bytes[end - 1] == b']' {
            return (end - 1, true, true);
        }
        (end, false, false)
    }

    /// The engine's `makeTag` five-state machine over one tag body:
    /// (1) tag name, then per parameter (2) name, (3) `=` search,
    /// (4) opening-quote detection, (5) value. Whitespace between the
    /// pieces becomes WHITESPACE trivia tokens; everything else keeps its
    /// raw text. Returns the tag name (empty when missing).
    fn lex_tag_body(&mut self, start: usize, end: usize, tag_end: TagEnd) -> String {
        let mut i = self.skip_ws(start, end);

        // State 1: tag name (until whitespace; the extent already excludes
        // the closing bracket for inline tags).
        let name_start = i;
        i = self.scan_until_ws(i, end);
        let name = self.src[name_start..i].to_string();
        self.emit_nonempty(SyntaxKind::IDENT, name_start, i);

        // Parameters.
        loop {
            i = self.skip_ws(i, end);
            if i >= end {
                break;
            }

            // Macro pass-through `*` (a lone star).
            if self.src.as_bytes()[i] == b'*' && is_ws_or_end(self.src, i + 1, end) {
                self.emit(SyntaxKind::STAR, i, i + 1);
                i += 1;
                continue;
            }

            // State 2: parameter name (until whitespace or `=`).
            let pname_start = i;
            while i < end {
                let b = self.src.as_bytes()[i];
                if b == b'=' || b.is_ascii_whitespace() {
                    break;
                }
                i += next_char_len(self.src, i);
            }
            self.emit_nonempty(SyntaxKind::IDENT, pname_start, i);

            // State 3: look for `=` (spaces around it are tolerated).
            let after_ws = self.skip_ws(i, end);
            if after_ws >= end || self.src.as_bytes()[after_ws] != b'=' {
                i = after_ws; // flag parameter (no value)
                continue;
            }
            self.emit(SyntaxKind::EQ, after_ws, after_ws + 1);
            i = self.skip_ws(after_ws + 1, end);
            if i >= end {
                break; // `param=` with no value
            }

            // States 4–5: quoted or unquoted value.
            let b = self.src.as_bytes()[i];
            if b == b'"' || b == b'\'' || b == b'`' {
                i = self.lex_quoted_value(i, end, b, tag_end);
            } else {
                i = self.lex_unquoted_value(i, end);
            }
        }
        name
    }

    /// A quoted parameter value, delimiters and escapes included verbatim.
    fn lex_quoted_value(&mut self, open: usize, end: usize, quote: u8, tag_end: TagEnd) -> usize {
        let bytes = self.src.as_bytes();
        let mut i = open + 1;
        while i < end {
            match bytes[i] {
                b'\\' => {
                    i += 1;
                    if i < end {
                        i += next_char_len(self.src, i);
                    }
                }
                b if b == quote => {
                    self.emit(SyntaxKind::STRING, open, i + 1);
                    return i + 1;
                }
                _ => i += next_char_len(self.src, i),
            }
        }
        // Unterminated quote. For inline tags the extent logic has already
        // decided whether a trailing `]` compensates (in which case `end`
        // excludes it); either way, the raw text to `end` is the value.
        let code = match tag_end {
            TagEnd::Bracket { compensated: true } => DiagCode::CompatCompensatedQuote,
            TagEnd::Bracket { compensated: false } | TagEnd::Eol => {
                DiagCode::LexUnterminatedString
            }
        };
        self.diag(Diagnostic::new(code, TextRange::new(size(open), size(end))));
        self.emit(SyntaxKind::STRING, open, end);
        end
    }

    /// An unquoted parameter value: runs to whitespace at bracket depth
    /// zero (nested `[]` pairs, as in `exp=f.a[0]`, stay inside the value).
    fn lex_unquoted_value(&mut self, start: usize, end: usize) -> usize {
        let bytes = self.src.as_bytes();
        let mut depth = 0usize;
        let mut i = start;
        while i < end {
            let b = bytes[i];
            match b {
                b'\\' => {
                    i += 1;
                    if i < end {
                        i += next_char_len(self.src, i);
                    }
                    continue;
                }
                b'[' => depth += 1,
                b']' if depth > 0 => depth -= 1,
                _ if b.is_ascii_whitespace() && depth == 0 => break,
                _ => {}
            }
            i += next_char_len(self.src, i);
        }
        let raw = &self.src[start..i];
        let kind = classify_unquoted(raw);
        self.emit_nonempty(kind, start, i);
        i
    }

    fn enter_block_mode_if_needed(&mut self, tag_name: &str) {
        match tag_name {
            "iscript" => self.mode = LexMode::IScript,
            "html" => self.mode = LexMode::Html,
            _ => {}
        }
    }

    // ---- low-level emission helpers -------------------------------------

    fn emit(&mut self, kind: SyntaxKind, start: usize, end: usize) {
        debug_assert_eq!(start, self.emitted, "tokens must be contiguous");
        debug_assert!(end >= start);
        if end > start {
            self.tokens.push(RawToken { kind, len: size(end - start) });
            self.emitted = end;
        }
    }

    /// Emits `kind` over `[start, end)`; empty ranges produce no token.
    fn emit_nonempty(&mut self, kind: SyntaxKind, start: usize, end: usize) {
        self.emit(kind, start, end);
    }

    /// Emits a WHITESPACE token over `[start, end)` if non-empty.
    fn ws(&mut self, start: usize, end: usize) {
        self.emit(SyntaxKind::WHITESPACE, start, end);
    }

    /// Emits WHITESPACE over any run of ASCII whitespace and returns the
    /// offset of the first non-whitespace byte.
    fn skip_ws(&mut self, start: usize, end: usize) -> usize {
        let bytes = self.src.as_bytes();
        let mut i = start;
        while i < end && bytes[i].is_ascii_whitespace() {
            i += 1;
        }
        self.ws(start, i);
        i
    }

    fn scan_until_ws(&self, start: usize, end: usize) -> usize {
        let bytes = self.src.as_bytes();
        let mut i = start;
        while i < end && !bytes[i].is_ascii_whitespace() {
            i += next_char_len(self.src, i);
        }
        i
    }

    fn diag(&mut self, d: Diagnostic) {
        self.diagnostics.push(d);
    }
}

/// Byte offset of the first non-whitespace char (whole-slice length if none).
fn offset_of_trimmed(s: &str) -> usize {
    s.len() - s.trim_start().len()
}

fn next_char_len(s: &str, i: usize) -> usize {
    if i >= s.len() {
        return 0;
    }
    let b = s.as_bytes()[i];
    if b < 0x80 { 1 } else { s[i..].chars().next().map_or(1, char::len_utf8) }
}

fn is_ws_or_end(s: &str, i: usize, end: usize) -> bool {
    i >= end || s.as_bytes()[i].is_ascii_whitespace()
}

/// The engine's unquoted-value classification: `&…` entity, `%…` macro
/// parameter reference, digits with at most one interior dot are numbers,
/// anything else is plain text.
fn classify_unquoted(raw: &str) -> SyntaxKind {
    if raw.starts_with('&') {
        return SyntaxKind::ENTITY;
    }
    if raw.starts_with('%') {
        return SyntaxKind::PARAM_REF;
    }
    if !raw.is_empty()
        && !raw.starts_with('.')
        && !raw.ends_with('.')
        && raw.chars().filter(|&c| c == '.').count() <= 1
        && raw.chars().all(|c| c.is_ascii_digit() || c == '.')
    {
        return SyntaxKind::NUMBER;
    }
    SyntaxKind::TEXT
}

/// True when a trimmed line begins with a real `[name …]` / `@name` tag
/// whose name is exactly `name` (the engine's exact-boundary check used to
/// terminate iscript/html blocks).
fn line_starts_with_tag(trimmed: &str, name: &str) -> bool {
    if let Some(rest) = trimmed.strip_prefix('[') {
        let rest = rest.trim_start();
        if let Some(after) = rest.strip_prefix(name) {
            match after.chars().next() {
                Some(c) if c.is_whitespace() || c == ']' => return true,
                _ => {}
            }
        }
    }
    if let Some(rest) = trimmed.strip_prefix('@') {
        let rest = rest.trim_start();
        if let Some(after) = rest.strip_prefix(name) {
            match after.chars().next() {
                None => return true,
                Some(c) if c.is_whitespace() => return true,
                _ => {}
            }
        }
    }
    false
}

#[inline]
fn size(n: usize) -> TextSize {
    TextSize::new(n as u32)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::kind::SyntaxKind as K;

    fn kinds(source: &str) -> Vec<SyntaxKind> {
        lex_kinds(source, &LexOptions::default())
    }

    fn lex_kinds(source: &str, opts: &LexOptions) -> Vec<SyntaxKind> {
        let out = lex(source, opts);
        let total: u32 = out.tokens.iter().map(|t| t.len.raw()).sum();
        assert_eq!(total as usize, source.len(), "coverage broken for {source:?}");
        out.tokens.iter().map(|t| t.kind).collect()
    }

    /// Reconstructs (kind, text) pairs for assertions on token boundaries.
    fn spell(source: &str) -> Vec<(SyntaxKind, String)> {
        let out = lex(source, &LexOptions::default());
        let mut pos = 0usize;
        out.tokens
            .iter()
            .map(|t| {
                let end = pos + t.len.raw() as usize;
                let text = source[pos..end].to_string();
                pos = end;
                (t.kind, text)
            })
            .collect()
    }

    #[test]
    fn empty_and_blank() {
        assert_eq!(kinds(""), vec![]);
        assert_eq!(kinds("\n"), vec![K::NEWLINE]);
        assert_eq!(kinds("   \n"), vec![K::WHITESPACE, K::NEWLINE]);
    }

    #[test]
    fn text_line_keeps_leading_ws_as_trivia() {
        assert_eq!(
            spell("  こんにちは  "),
            vec![(K::WHITESPACE, "  ".into()), (K::TEXT, "こんにちは  ".into())]
        );
    }

    #[test]
    fn underscore_preserves_following_space() {
        assert_eq!(
            spell("  _  hi"),
            vec![
                (K::WHITESPACE, "  ".into()),
                (K::UNDERSCORE, "_".into()),
                (K::TEXT, "  hi".into()),
            ]
        );
    }

    #[test]
    fn inline_tags_split_text() {
        assert_eq!(
            kinds("こんにちは[l]世界"),
            vec![K::TEXT, K::L_BRACKET, K::IDENT, K::R_BRACKET, K::TEXT]
        );
        // A bare space between two tags is text content.
        assert_eq!(
            kinds("[l] [r]"),
            vec![
                K::L_BRACKET,
                K::IDENT,
                K::R_BRACKET,
                K::TEXT,
                K::L_BRACKET,
                K::IDENT,
                K::R_BRACKET
            ]
        );
    }

    #[test]
    fn escaped_bracket_is_text() {
        assert_eq!(spell(r"\[not a tag\]"), vec![(K::TEXT, r"\[not a tag\]".into())]);
    }

    #[test]
    fn tag_with_params() {
        assert_eq!(
            spell("[bg storage=room.jpg time=1000]"),
            vec![
                (K::L_BRACKET, "[".into()),
                (K::IDENT, "bg".into()),
                (K::WHITESPACE, " ".into()),
                (K::IDENT, "storage".into()),
                (K::EQ, "=".into()),
                (K::TEXT, "room.jpg".into()),
                (K::WHITESPACE, " ".into()),
                (K::IDENT, "time".into()),
                (K::EQ, "=".into()),
                (K::NUMBER, "1000".into()),
                (K::R_BRACKET, "]".into()),
            ]
        );
    }

    #[test]
    fn quoted_value_keeps_quotes() {
        assert_eq!(
            spell(r#"@jump storage="title.ks""#),
            vec![
                (K::AT, "@".into()),
                (K::IDENT, "jump".into()),
                (K::WHITESPACE, " ".into()),
                (K::IDENT, "storage".into()),
                (K::EQ, "=".into()),
                (K::STRING, "\"title.ks\"".into()),
            ]
        );
    }

    #[test]
    fn spaces_around_eq_are_trivia() {
        assert_eq!(
            kinds("[a p = 1]"),
            vec![
                K::L_BRACKET,
                K::IDENT,
                K::WHITESPACE,
                K::IDENT,
                K::WHITESPACE,
                K::EQ,
                K::WHITESPACE,
                K::NUMBER,
                K::R_BRACKET
            ]
        );
    }

    #[test]
    fn macro_star_and_flag_params() {
        assert_eq!(
            kinds("[macro_use * flag2]"),
            vec![
                K::L_BRACKET,
                K::IDENT,
                K::WHITESPACE,
                K::STAR,
                K::WHITESPACE,
                K::IDENT,
                K::R_BRACKET
            ]
        );
    }

    #[test]
    fn entity_and_param_ref_values() {
        assert_eq!(
            kinds("[eval exp=&f.name cond=%flag]"),
            vec![
                K::L_BRACKET,
                K::IDENT,
                K::WHITESPACE,
                K::IDENT,
                K::EQ,
                K::ENTITY,
                K::WHITESPACE,
                K::IDENT,
                K::EQ,
                K::PARAM_REF,
                K::R_BRACKET
            ]
        );
    }

    #[test]
    fn nested_brackets_in_unquoted_value() {
        assert_eq!(
            spell("[eval exp=f.a[0]]"),
            vec![
                (K::L_BRACKET, "[".into()),
                (K::IDENT, "eval".into()),
                (K::WHITESPACE, " ".into()),
                (K::IDENT, "exp".into()),
                (K::EQ, "=".into()),
                (K::TEXT, "f.a[0]".into()),
                (K::R_BRACKET, "]".into()),
            ]
        );
    }

    #[test]
    fn double_bracket_in_quoted_value() {
        assert_eq!(
            spell(r#"[ptext text="[[あ]]"]"#),
            vec![
                (K::L_BRACKET, "[".into()),
                (K::IDENT, "ptext".into()),
                (K::WHITESPACE, " ".into()),
                (K::IDENT, "text".into()),
                (K::EQ, "=".into()),
                (K::STRING, "\"[[あ]]\"".into()),
                (K::R_BRACKET, "]".into()),
            ]
        );
    }

    #[test]
    fn quote_compensation() {
        let out = lex(r#"[ptext text="abc]"#, &LexOptions::default());
        let toks: Vec<_> = out.tokens.iter().map(|t| t.kind).collect();
        assert_eq!(
            toks,
            vec![K::L_BRACKET, K::IDENT, K::WHITESPACE, K::IDENT, K::EQ, K::STRING, K::R_BRACKET]
        );
        assert!(out.diagnostics.iter().any(|d| matches!(d.code, DiagCode::CompatCompensatedQuote)));
    }

    #[test]
    fn unterminated_inline_tag() {
        let out = lex("[ptext text=abc", &LexOptions::default());
        let toks: Vec<_> = out.tokens.iter().map(|t| t.kind).collect();
        assert_eq!(toks, vec![K::L_BRACKET, K::IDENT, K::WHITESPACE, K::IDENT, K::EQ, K::TEXT]);
        assert!(out.diagnostics.iter().any(|d| matches!(d.code, DiagCode::ParseUnterminatedTag)));
    }

    #[test]
    fn label_and_chara_lines() {
        assert_eq!(kinds("*start"), vec![K::STAR, K::TEXT]);
        assert_eq!(kinds("*oped|オープニング"), vec![K::STAR, K::TEXT, K::PIPE, K::TEXT]);
        // The extra segment stays in the raw value token (interpretation
        // truncates later; the tree never loses it).
        assert_eq!(
            spell("*a|b|c")[2..].to_vec(),
            vec![(K::PIPE, "|".into()), (K::TEXT, "b|c".into())]
        );
        assert_eq!(kinds("#akane:happy"), vec![K::SHARP, K::TEXT, K::COLON, K::TEXT]);
        assert_eq!(kinds("#"), vec![K::SHARP]);
        assert_eq!(spell("#a:b:c")[3].1, "b:c");
        // Empty leading segment keeps its separator position.
        assert_eq!(kinds("#:face"), vec![K::SHARP, K::COLON, K::TEXT]);
    }

    #[test]
    fn comment_lines() {
        assert_eq!(kinds(";コメント"), vec![K::SEMICOLON, K::COMMENT_TEXT]);
        // `;` mid-line is plain text.
        assert_eq!(kinds("a;b"), vec![K::TEXT]);
    }

    #[test]
    fn block_comment_keeps_interior() {
        let src = "/*\nhidden line\n  */\ntext";
        assert_eq!(
            spell(src),
            vec![
                (K::SLASH_STAR, "/*".into()),
                (K::NEWLINE, "\n".into()),
                (K::COMMENT_TEXT, "hidden line".into()),
                (K::NEWLINE, "\n".into()),
                (K::WHITESPACE, "  ".into()),
                (K::STAR_SLASH, "*/".into()),
                (K::NEWLINE, "\n".into()),
                (K::TEXT, "text".into()),
            ]
        );
    }

    #[test]
    fn orphan_close_is_label() {
        // `*/` outside a block comment is a label named `/` (engine quirk).
        assert_eq!(spell("*/"), vec![(K::STAR, "*".into()), (K::TEXT, "/".into())]);
        // `hoge */` does NOT close a block comment.
        let src = "/*\nhoge */\n*/\n";
        let out = lex(src, &LexOptions::default());
        let toks: Vec<_> = out.tokens.iter().map(|t| t.kind).collect();
        assert_eq!(
            toks,
            vec![K::SLASH_STAR, K::NEWLINE, K::COMMENT_TEXT, K::NEWLINE, K::STAR_SLASH, K::NEWLINE]
        );
    }

    #[test]
    fn iscript_block_roundtrip() {
        let src = "[iscript]\n  var a = 1;\n\n[endscript]";
        assert_eq!(
            spell(src),
            vec![
                (K::L_BRACKET, "[".into()),
                (K::IDENT, "iscript".into()),
                (K::R_BRACKET, "]".into()),
                (K::NEWLINE, "\n".into()),
                (K::SCRIPT_TEXT, "  var a = 1;".into()),
                (K::NEWLINE, "\n".into()),
                (K::NEWLINE, "\n".into()),
                (K::L_BRACKET, "[".into()),
                (K::IDENT, "endscript".into()),
                (K::R_BRACKET, "]".into()),
            ]
        );
    }

    #[test]
    fn loose_endscript_reparses_line() {
        let src = "[iscript]\nvar s = \"endscript\";\n[s]";
        let out = lex(src, &LexOptions::default());
        let toks: Vec<_> = out.tokens.iter().map(|t| t.kind).collect();
        assert!(toks.contains(&K::TEXT));
        assert!(out.diagnostics.iter().any(|d| matches!(d.code, DiagCode::CompatLooseEndscript)));
        // Strict mode: the line stays script text and the block never ends.
        let strict = LexOptions { loose_endscript_termination: false };
        let out = lex(src, &strict);
        let script_lines = out.tokens.iter().filter(|t| t.kind == K::SCRIPT_TEXT).count();
        assert_eq!(script_lines, 2);
        assert!(
            out.diagnostics.iter().any(|d| matches!(d.code, DiagCode::LexUnterminatedIscript))
        );
    }

    #[test]
    fn endscript_boundary_is_exact() {
        let strict = LexOptions { loose_endscript_termination: false };
        // `[endscript2]` does not close; `[endscript foo=1]` does.
        let src = "[iscript]\n[endscript2]\n[endscript foo=1]\n";
        let out = lex(src, &strict);
        assert_eq!(out.line_modes, vec![LexMode::Default, LexMode::IScript, LexMode::IScript]);
        assert!(
            !out.diagnostics.iter().any(|d| matches!(d.code, DiagCode::LexUnterminatedIscript))
        );
        // `[ endscript]` with a leading space also closes.
        let out2 = lex("[iscript]\n[ endscript]\n", &strict);
        assert!(
            !out2.diagnostics.iter().any(|d| matches!(d.code, DiagCode::LexUnterminatedIscript))
        );
    }

    #[test]
    fn at_iscript_line_is_preserved() {
        // The old lexer swallowed the `@iscript` line entirely; the new one
        // keeps its tokens and still switches modes.
        let src = "@iscript\ncode\n@endscript\n";
        assert_eq!(
            kinds(src),
            vec![
                K::AT,
                K::IDENT,
                K::NEWLINE,
                K::SCRIPT_TEXT,
                K::NEWLINE,
                K::AT,
                K::IDENT,
                K::NEWLINE
            ]
        );
    }

    #[test]
    fn html_block() {
        let src = "[html]\n<b>bold</b>\n[endhtml]\n";
        assert_eq!(
            kinds(src),
            vec![
                K::L_BRACKET,
                K::IDENT,
                K::R_BRACKET,
                K::NEWLINE,
                K::HTML_TEXT,
                K::NEWLINE,
                K::L_BRACKET,
                K::IDENT,
                K::R_BRACKET,
                K::NEWLINE
            ]
        );
        // `[html2]` is an ordinary tag, not a block opener.
        let out = lex("[html2]\ntext\n", &LexOptions::default());
        assert_eq!(out.line_modes, vec![LexMode::Default, LexMode::Default]);
    }

    #[test]
    fn inline_iscript_rest_of_line_is_script() {
        assert_eq!(
            kinds("[iscript]var a=1;"),
            vec![K::L_BRACKET, K::IDENT, K::R_BRACKET, K::SCRIPT_TEXT]
        );
    }

    #[test]
    fn crlf_newlines() {
        assert_eq!(
            spell("a\r\nb\n"),
            vec![
                (K::TEXT, "a".into()),
                (K::NEWLINE, "\r\n".into()),
                (K::TEXT, "b".into()),
                (K::NEWLINE, "\n".into()),
            ]
        );
    }

    #[test]
    fn bom_is_trivia_token() {
        assert_eq!(spell("\u{feff}text"), vec![(K::BOM, "\u{feff}".into()), (K::TEXT, "text".into())]);
    }

    #[test]
    fn digit_leading_tag_name() {
        assert_eq!(kinds("[3d_init]"), vec![K::L_BRACKET, K::IDENT, K::R_BRACKET]);
    }

    #[test]
    fn empty_value_param() {
        assert_eq!(
            kinds("[a t=]"),
            vec![K::L_BRACKET, K::IDENT, K::WHITESPACE, K::IDENT, K::EQ, K::R_BRACKET]
        );
    }

    #[test]
    fn line_modes_track_blocks() {
        let src = "text\n[iscript]\ncode\n[endscript]\ntext\n";
        let out = lex(src, &LexOptions::default());
        assert_eq!(
            out.line_modes,
            vec![
                LexMode::Default,
                LexMode::Default,
                LexMode::IScript,
                LexMode::IScript,
                LexMode::Default,
            ]
        );
    }

    #[test]
    fn no_panic_on_odd_inputs() {
        for src in [
            "\\",
            "[",
            "]",
            "[]",
            "[ ]",
            "@",
            "@ ",
            "*",
            "#\n",
            "_",
            ";",
            "/*",
            "*/x",
            "[a=b=c]",
            "[a \"q]",
            "\u{feff}",
            "\r",
            "a\rb",
        ] {
            let out = lex(src, &LexOptions::default());
            let total: u32 = out.tokens.iter().map(|t| t.len.raw()).sum();
            assert_eq!(total as usize, src.len(), "coverage broken for {src:?}");
        }
    }
}
