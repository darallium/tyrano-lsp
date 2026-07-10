//! Line-oriented, context-sensitive scanner for TyranoScript.
//!
//! The tokenization mirrors the reference implementation in
//! `tyrano/plugins/kag/kag.parser.js` (`parseScenario` + `makeTag`):
//! every line is trimmed and dispatched on its first character, text lines
//! are scanned char-by-char with quote/bracket-depth aware inline tags, and
//! tag bodies are tokenized with the same five-state machine the engine uses.

use super::token::{Token, TokenType};
use crate::Result;
use crate::config::{KeepSpaceLevel, ParserConfig};

/// Scanner operating mode for handling special blocks
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ScannerMode {
    /// Normal TyranoScript parsing
    Default,
    /// Inside [iscript]...[endscript] block
    Iscript,
    /// Inside [html]...[endhtml] block
    Html,
}

pub struct Scanner {
    input: String,
    mode: ScannerMode,
    flag_comment: bool,
    config: ParserConfig,
}

impl Scanner {
    pub fn new(input: &str) -> Self {
        Self::with_config(input, ParserConfig::default())
    }

    pub fn with_config(input: &str, config: ParserConfig) -> Self {
        Scanner {
            input: input.to_string(),
            mode: ScannerMode::Default,
            flag_comment: false,
            config,
        }
    }

    pub fn scan_tokens(&mut self) -> Result<Vec<Token>> {
        let mut tokens = Vec::new();
        let input = std::mem::take(&mut self.input);

        let mut lines: Vec<&str> = input.split('\n').collect();
        // A trailing newline yields a final empty segment that is not a line
        // of its own; drop it but remember every '\n' emits a Newline token.
        let had_trailing_segment = lines.last() == Some(&"");
        if had_trailing_segment {
            lines.pop();
        }
        let line_count = lines.len();

        for (idx, raw_line) in lines.into_iter().enumerate() {
            let line_no = idx + 1;
            self.lex_line(raw_line, line_no, &mut tokens);
            let has_newline = idx + 1 < line_count || had_trailing_segment;
            if has_newline {
                tokens.push(Token::new(TokenType::Newline, line_no, raw_line.len() + 1));
            }
        }

        // Close unterminated blocks so the grammar always sees matched pairs.
        match self.mode {
            ScannerMode::Iscript => {
                tokens.push(Token::new(TokenType::IscriptEnd, line_count.max(1), 1));
            }
            ScannerMode::Html => {
                tokens.push(Token::new(TokenType::HtmlEnd, line_count.max(1), 1));
            }
            ScannerMode::Default => {}
        }

        tokens.push(Token::new(TokenType::Eof, line_count.max(1), 1));
        self.input = input;

        Ok(tokens)
    }

    fn lex_line(&mut self, raw_line: &str, line_no: usize, tokens: &mut Vec<Token>) {
        let trimmed = raw_line.trim();

        match self.mode {
            ScannerMode::Iscript => self.lex_iscript_line(trimmed, line_no, tokens),
            ScannerMode::Html => self.lex_html_line(trimmed, line_no, tokens),
            ScannerMode::Default => self.dispatch_line(trimmed, line_no, tokens),
        }
    }

    /// A line inside an [iscript] block.
    fn lex_iscript_line(&mut self, trimmed: &str, line_no: usize, tokens: &mut Vec<Token>) {
        let is_end_tag_line = line_starts_with_tag(trimmed, "endscript");
        let ends_block = if self.config.loose_endscript_termination {
            // 悪法: the engine checks `line.indexOf("endscript") != -1`.
            trimmed.contains("endscript")
        } else {
            is_end_tag_line
        };

        if !ends_block {
            if !trimmed.is_empty() {
                tokens.push(Token::new(
                    TokenType::ScriptText(trimmed.to_string()),
                    line_no,
                    1,
                ));
            }
            return;
        }

        self.mode = ScannerMode::Default;
        tokens.push(Token::new(TokenType::IscriptEnd, line_no, 1));

        if is_end_tag_line {
            // Consume the closing tag itself; anything after it on the same
            // line is ordinary scenario content.
            let rest = if trimmed.starts_with('[') {
                trimmed.split_once(']').map(|(_, r)| r).unwrap_or("")
            } else {
                // @endscript: the whole line belongs to the tag.
                ""
            };
            if !rest.is_empty() {
                self.lex_text_content(rest, line_no, tokens);
            }
        } else {
            // Loose termination: the line itself is parsed as a normal line.
            self.dispatch_line(trimmed, line_no, tokens);
        }
    }

    /// A line inside an [html] block.
    fn lex_html_line(&mut self, trimmed: &str, line_no: usize, tokens: &mut Vec<Token>) {
        if line_starts_with_tag(trimmed, "endhtml") {
            self.mode = ScannerMode::Default;
            tokens.push(Token::new(TokenType::HtmlEnd, line_no, 1));
            let rest = if trimmed.starts_with('[') {
                trimmed.split_once(']').map(|(_, r)| r).unwrap_or("")
            } else {
                ""
            };
            if !rest.is_empty() {
                self.lex_text_content(rest, line_no, tokens);
            }
            return;
        }

        if !trimmed.is_empty() {
            tokens.push(Token::new(
                TokenType::HtmlText(trimmed.to_string()),
                line_no,
                1,
            ));
        }
    }

    /// Dispatch a trimmed line on its first character, like `parseScenario`.
    fn dispatch_line(&mut self, trimmed: &str, line_no: usize, tokens: &mut Vec<Token>) {
        if self.flag_comment {
            // Only a lone "*/" line leaves a block comment.
            if trimmed == "*/" {
                self.flag_comment = false;
                tokens.push(Token::new(TokenType::BlockCommentEnd, line_no, 1));
            }
            return;
        }

        if trimmed == "/*" {
            self.flag_comment = true;
            tokens.push(Token::new(TokenType::BlockCommentStart, line_no, 1));
            return;
        }

        if trimmed.is_empty() {
            return;
        }

        let first = trimmed.chars().next().unwrap();
        match first {
            ';' => {
                tokens.push(Token::new(
                    TokenType::LineComment(trimmed.to_string()),
                    line_no,
                    1,
                ));
            }
            '#' => self.lex_chara_line(trimmed, line_no, tokens),
            '*' => self.lex_label_line(trimmed, line_no, tokens),
            '@' => {
                let body = &trimmed[1..];
                let name = tag_name_of(body);
                if name == "iscript" {
                    self.mode = ScannerMode::Iscript;
                    tokens.push(Token::new(TokenType::IscriptStart, line_no, 1));
                } else if name == "html" {
                    self.mode = ScannerMode::Html;
                    tokens.push(Token::new(TokenType::HtmlStart, line_no, 1));
                } else {
                    tokens.push(Token::new(TokenType::At, line_no, 1));
                    self.lex_tag_body(body, line_no, tokens);
                }
            }
            '_' => {
                tokens.push(Token::new(TokenType::Underscore, line_no, 1));
                // The trim already happened, so whitespace after the "_"
                // marker survives (that is the marker's whole purpose).
                let rest = &trimmed[1..];
                if !rest.is_empty() {
                    self.lex_text_content(rest, line_no, tokens);
                }
            }
            _ => self.lex_text_content(trimmed, line_no, tokens),
        }
    }

    /// `#name:face` — the engine trims the whole remainder but not the
    /// individual segments, and only `split(":")[0..=1]` survive.
    fn lex_chara_line(&mut self, trimmed: &str, line_no: usize, tokens: &mut Vec<Token>) {
        tokens.push(Token::new(TokenType::Sharp, line_no, 1));
        let rest = trimmed[1..].trim();
        if rest.is_empty() {
            return;
        }

        match rest.split_once(':') {
            Some((name, face_full)) => {
                if !name.is_empty() {
                    tokens.push(Token::new(TokenType::Text(name.to_string()), line_no, 2));
                }
                tokens.push(Token::new(TokenType::Colon, line_no, 2 + name.len()));
                let face = if self.config.chara_face_first_segment_only {
                    face_full.split(':').next().unwrap_or("")
                } else {
                    face_full
                };
                if !face.is_empty() {
                    tokens.push(Token::new(
                        TokenType::Text(face.to_string()),
                        line_no,
                        3 + name.len(),
                    ));
                }
            }
            None => {
                tokens.push(Token::new(TokenType::Text(rest.to_string()), line_no, 2));
            }
        }
    }

    /// `*label|value` — both segments are trimmed; extra `|` segments are
    /// dropped by the engine.
    fn lex_label_line(&mut self, trimmed: &str, line_no: usize, tokens: &mut Vec<Token>) {
        tokens.push(Token::new(TokenType::Asterisk, line_no, 1));
        let rest = &trimmed[1..];

        match rest.split_once('|') {
            Some((key_raw, val_full)) => {
                let key = key_raw.trim();
                if !key.is_empty() {
                    tokens.push(Token::new(TokenType::Text(key.to_string()), line_no, 2));
                }
                tokens.push(Token::new(TokenType::Pipe, line_no, 2 + key_raw.len()));
                let val_raw = if self.config.label_value_first_segment_only {
                    val_full.split('|').next().unwrap_or("")
                } else {
                    val_full
                };
                let val = val_raw.trim();
                if !val.is_empty() {
                    tokens.push(Token::new(
                        TokenType::Text(val.to_string()),
                        line_no,
                        3 + key_raw.len(),
                    ));
                }
            }
            None => {
                let key = rest.trim();
                if !key.is_empty() {
                    tokens.push(Token::new(TokenType::Text(key.to_string()), line_no, 2));
                }
            }
        }
    }

    /// Char-by-char scan of a text line (or remainder), handling `\` escapes
    /// and inline `[tag]`s with the engine's quote/bracket-depth rules.
    fn lex_text_content(&mut self, content: &str, line_no: usize, tokens: &mut Vec<Token>) {
        let chars: Vec<char> = content.chars().collect();
        let mut i = 0;
        let mut text = String::new();

        while i < chars.len() {
            let c = chars[i];
            if c == '\\' {
                // Escape: take the next char literally (a trailing backslash
                // is silently dropped, like the engine's flag_escape).
                if i + 1 < chars.len() {
                    text.push(chars[i + 1]);
                }
                i += 2;
                continue;
            }
            if c == '[' {
                if !text.is_empty() {
                    tokens.push(Token::new(TokenType::Text(std::mem::take(&mut text)), line_no, 1));
                }

                // Extract the tag body up to the matching ']' at depth 0,
                // honouring quotes exactly like parseScenario.
                let mut body = String::new();
                let mut depth = 1usize;
                let mut start_quot: Option<char> = None;
                let mut j = i + 1;
                let mut closed = false;
                while j < chars.len() {
                    let t = chars[j];
                    match t {
                        ']' => {
                            if start_quot.is_some() {
                                body.push(t);
                            } else {
                                depth -= 1;
                                if depth == 0 {
                                    closed = true;
                                    j += 1;
                                    break;
                                }
                                body.push(t);
                            }
                        }
                        '[' => {
                            if start_quot.is_none() {
                                depth += 1;
                            }
                            body.push(t);
                        }
                        '"' | '\'' | '`' => {
                            match start_quot {
                                Some(q) if q == t => start_quot = None,
                                None => start_quot = Some(t),
                                _ => {}
                            }
                            body.push(t);
                        }
                        _ => body.push(t),
                    }
                    j += 1;
                }
                if !closed && body.ends_with(']') {
                    // "compensate_missing_quart": an unclosed quote swallowed
                    // the final ']' — strip it and complete the tag anyway.
                    body.pop();
                }
                i = j;

                let name = tag_name_of(&body);
                if name == "iscript" {
                    self.mode = ScannerMode::Iscript;
                    tokens.push(Token::new(TokenType::IscriptStart, line_no, 1));
                    // The rest of the line is script text (the engine feeds
                    // it through flag_script).
                    let rest: String = chars[i..].iter().collect();
                    let rest = rest.trim();
                    if !rest.is_empty() {
                        tokens.push(Token::new(
                            TokenType::ScriptText(rest.to_string()),
                            line_no,
                            1,
                        ));
                    }
                    return;
                } else if name == "html" {
                    self.mode = ScannerMode::Html;
                    tokens.push(Token::new(TokenType::HtmlStart, line_no, 1));
                    let rest: String = chars[i..].iter().collect();
                    let rest = rest.trim();
                    if !rest.is_empty() {
                        tokens.push(Token::new(TokenType::HtmlText(rest.to_string()), line_no, 1));
                    }
                    return;
                }

                tokens.push(Token::new(TokenType::LBracket, line_no, 1));
                self.lex_tag_body(&body, line_no, tokens);
                tokens.push(Token::new(TokenType::RBracket, line_no, 1));
                continue;
            }

            text.push(c);
            i += 1;
        }

        if !text.is_empty() {
            tokens.push(Token::new(TokenType::Text(text), line_no, 1));
        }
    }

    /// Tokenize the inside of a tag with `makeTag`'s five-state machine.
    /// `body` excludes the surrounding brackets / the leading `@`.
    fn lex_tag_body(&mut self, body: &str, line_no: usize, tokens: &mut Vec<Token>) {
        let chars: Vec<char> = body.chars().collect();
        let mut i = 0;

        // --- tag name: skip leading spaces, then take everything until a space
        while i < chars.len() && chars[i] == ' ' {
            i += 1;
        }
        let mut name = String::new();
        while i < chars.len() && chars[i] != ' ' {
            name.push(chars[i]);
            i += 1;
        }
        tokens.push(Token::new(TokenType::Identifier(name), line_no, 1));

        // --- parameters
        loop {
            while i < chars.len() && chars[i] == ' ' {
                i += 1;
            }
            if i >= chars.len() {
                break;
            }

            // Parameter name: until ' ' or '='.
            let mut pname = String::new();
            while i < chars.len() && chars[i] != ' ' && chars[i] != '=' {
                pname.push(chars[i]);
                i += 1;
            }

            // SCANNING_EQUAL: skip spaces, then look for '='.
            let mut k = i;
            while k < chars.len() && chars[k] == ' ' {
                k += 1;
            }
            let has_equal = k < chars.len() && chars[k] == '=';

            let name_token = if pname == "*" {
                TokenType::Asterisk
            } else {
                TokenType::Identifier(pname)
            };
            tokens.push(Token::new(name_token, line_no, 1));

            if !has_equal {
                // Flag parameter (or '*' pass-through); the next non-space
                // char starts a new parameter name.
                i = k;
                continue;
            }

            // Consume '='.
            i = k + 1;
            tokens.push(Token::new(TokenType::Equal, line_no, 1));

            // SCANNING_START_QUOT: skip spaces before the value.
            while i < chars.len() && chars[i] == ' ' {
                i += 1;
            }
            if i >= chars.len() {
                // "name=" at the very end: the engine registers an empty
                // value; the grammar models this as `param_name EQUAL`.
                break;
            }

            let quote = matches!(chars[i], '"' | '\'' | '`').then(|| chars[i]);
            if let Some(q) = quote {
                i += 1;
                let (value, next) = self.read_param_value(&chars, i, Some(q));
                i = next;
                tokens.push(Token::new(
                    TokenType::String(finalize_param_value(value, &self.config)),
                    line_no,
                    1,
                ));
            } else {
                let (value, next) = self.read_param_value(&chars, i, None);
                i = next;
                let value = finalize_param_value(value, &self.config);
                tokens.push(Token::new(classify_unquoted(value), line_no, 1));
            }
        }
    }

    /// Read a parameter value until the end char (the opening quote, or a
    /// space for unquoted values), honouring `\` escapes and the
    /// KeepSpaceInParameterValue space-removal rule.
    fn read_param_value(
        &self,
        chars: &[char],
        mut i: usize,
        quote: Option<char>,
    ) -> (String, usize) {
        let end_char = quote.unwrap_or(' ');
        let remove_spaces = self.config.keep_space_in_parameter_value == KeepSpaceLevel::RemoveAll
            && quote != Some('`');
        let mut value = String::new();
        let mut escape = false;

        while i < chars.len() {
            let mut c = chars[i];
            if c == end_char && !escape {
                i += 1; // consume the terminator
                return (value, i);
            }
            // The engine blanks spaces (level 1) before the escape check,
            // so even escaped spaces disappear at that level.
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

        (value, i)
    }
}

/// Final value processing from `makeParam`: trim (unless KeepAll) and map a
/// trimmed "undefined" to the empty string.
fn finalize_param_value(value: String, config: &ParserConfig) -> String {
    let trimmed = value.trim();
    if trimmed == "undefined" {
        return String::new();
    }
    if config.keep_space_in_parameter_value == KeepSpaceLevel::KeepAll {
        value
    } else {
        trimmed.to_string()
    }
}

/// Classify an unquoted parameter value into the token the grammar expects.
fn classify_unquoted(value: String) -> TokenType {
    if value.starts_with('&') {
        return TokenType::Entity(value);
    }
    if value.starts_with('%') {
        return TokenType::ParamRef(value);
    }
    let mut digits = 0usize;
    let mut dots = 0usize;
    let numeric = !value.is_empty()
        && value.chars().all(|c| {
            if c.is_ascii_digit() {
                digits += 1;
                true
            } else if c == '.' {
                dots += 1;
                true
            } else {
                false
            }
        });
    if numeric && digits > 0 && dots <= 1 && !value.starts_with('.') && !value.ends_with('.') {
        return TokenType::Number(value);
    }
    TokenType::Text(value)
}

/// First space-delimited word of a tag body (after leading spaces) — the
/// tag's name as `makeTag` would compute it.
fn tag_name_of(body: &str) -> &str {
    body.trim_start_matches(' ')
        .split(' ')
        .next()
        .unwrap_or("")
}

/// True if the trimmed line begins with a `[...]` or `@` tag whose makeTag
/// name is exactly `name`. Requires a real tag-name boundary, so prefix
/// look-alikes such as `[endscript2]` do not match `endscript`. Mirrors the
/// engine: spaces before the name are skipped, the name ends at a space, and
/// `]` closes a bracket tag.
fn line_starts_with_tag(trimmed: &str, name: &str) -> bool {
    let Some(first) = trimmed.chars().next() else {
        return false;
    };
    let body = &trimmed[1..];
    let tag = match first {
        // Inside brackets the name additionally ends at the closing ']'.
        '[' => body
            .trim_start_matches(' ')
            .split([' ', ']'])
            .next()
            .unwrap_or(""),
        '@' => tag_name_of(body),
        _ => return false,
    };
    tag == name
}
