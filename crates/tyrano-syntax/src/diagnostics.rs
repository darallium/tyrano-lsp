//! Structured, localizable diagnostics.
//!
//! The lexer, parser, and validators never build human-readable strings.
//! They emit [`Diagnostic`] values that carry a stable [`DiagCode`], the
//! source [`TextRange`]s involved, and structured arguments. The message
//! *text* is produced later, on demand, by [`render`] for a chosen [`Lang`],
//! so the same diagnostic can be shown in English or Japanese (or dumped to
//! a machine-readable format) without the front end knowing about locales.
//!
//! Codes are the stable contract: [`DiagCode::as_str`] returns a fixed
//! `E_…`/`W_…` identifier suitable for suppression comments, golden files,
//! and editor problem-matchers.

use crate::kind::SyntaxKind;
use crate::text::{SourceText, TextRange, TextSize};

// ======================================================================
// Severity
// ======================================================================

/// How serious a diagnostic is.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum Severity {
    /// A hard problem: invalid syntax that the recovery machinery had to
    /// paper over.
    Error,
    /// A compatibility quirk or a lint: the input parses, but with a caveat
    /// worth surfacing.
    Warning,
    /// Purely informational.
    Info,
}

impl Severity {
    /// The uppercase label used in `line:col: SEVERITY CODE: message`
    /// rendering.
    pub const fn label(self) -> &'static str {
        match self {
            Severity::Error => "error",
            Severity::Warning => "warning",
            Severity::Info => "info",
        }
    }
}

// ======================================================================
// DiagCode
// ======================================================================

/// A fixed, stable diagnostic code.
///
/// The set is closed and each variant maps to exactly one `E_…`/`W_…`
/// identifier via [`DiagCode::as_str`] and one [`Severity`] via
/// [`DiagCode::default_severity`]. `W_` codes are warnings (compatibility
/// interpretations); everything else defaults to an error.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum DiagCode {
    // -- lexical -------------------------------------------------------
    /// A quoted string ran to end of line / end of file without a closer.
    LexUnterminatedString,
    /// An `[iscript]` block reached EOF without `[endscript]`.
    LexUnterminatedIscript,
    /// An `[html]` block reached EOF without `[endhtml]`.
    LexUnterminatedHtml,
    /// A `/* … */` block comment reached EOF without `*/`.
    LexUnterminatedBlockComment,
    /// A lone `\` sat at end of line where an escape target was expected.
    LexStrayBackslashAtEol,
    /// Bytes that are not valid in the current lexical context.
    LexInvalidBytes,

    // -- parse ---------------------------------------------------------
    /// A specific token was expected here. Expected kinds are in `expected`.
    ParseExpectedToken,
    /// A tag name was expected after `@` or `[`.
    ParseExpectedTagName,
    /// A token appeared where nothing valid could continue the construct.
    ParseUnexpectedToken,
    /// A tag was opened but never closed. Secondary points at the opener.
    ParseUnterminatedTag,

    // -- expression sub-language --------------------------------------
    /// An operand (literal, name, or parenthesised expression) was expected.
    ExprExpectedOperand,
    /// A specific expression token was expected. See `expected`.
    ExprExpectedToken,
    /// A `(` was never matched by a `)`, or vice versa.
    ExprUnbalancedParen,
    /// Input remained after a complete expression was parsed.
    ExprTrailingInput,

    // -- compat interpretations (warnings) ----------------------------
    /// `t="undefined"` was cooked to the empty string, as the engine does.
    CompatUndefinedValue,
    /// A label line carried an extra `|`-segment that the engine drops.
    CompatLabelExtraSegmentDropped,
    /// A character line carried an extra `:`-segment that the engine drops.
    CompatCharaExtraSegmentDropped,
    /// A block was ended by a line merely *containing* `endscript`.
    CompatLooseEndscript,
    /// A missing closing quote was compensated the way the engine does.
    CompatCompensatedQuote,

    // -- validation ----------------------------------------------------
    /// The same parameter name appeared twice on one tag.
    ValidDuplicateParam,
    /// A tag had no name at all.
    ValidEmptyTagName,

    // -- semantic (used by tyrano-analysis) ---------------------------
    /// A jump/call referenced a label that does not exist.
    SemUnknownLabel,
    /// The same label name was defined more than once.
    SemDuplicateLabel,
}

impl DiagCode {
    /// The stable string identifier for this code.
    ///
    /// Warnings use a `W_` prefix; all other codes use `E_`.
    pub const fn as_str(&self) -> &'static str {
        match self {
            // lexical
            DiagCode::LexUnterminatedString => "E_LEX_UNTERMINATED_STRING",
            DiagCode::LexUnterminatedIscript => "E_LEX_UNTERMINATED_ISCRIPT",
            DiagCode::LexUnterminatedHtml => "E_LEX_UNTERMINATED_HTML",
            DiagCode::LexUnterminatedBlockComment => "E_LEX_UNTERMINATED_BLOCK_COMMENT",
            DiagCode::LexStrayBackslashAtEol => "E_LEX_STRAY_BACKSLASH_AT_EOL",
            DiagCode::LexInvalidBytes => "E_LEX_INVALID_BYTES",
            // parse
            DiagCode::ParseExpectedToken => "E_PARSE_EXPECTED_TOKEN",
            DiagCode::ParseExpectedTagName => "E_PARSE_EXPECTED_TAG_NAME",
            DiagCode::ParseUnexpectedToken => "E_PARSE_UNEXPECTED_TOKEN",
            DiagCode::ParseUnterminatedTag => "E_PARSE_UNTERMINATED_TAG",
            // expression
            DiagCode::ExprExpectedOperand => "E_EXPR_EXPECTED_OPERAND",
            DiagCode::ExprExpectedToken => "E_EXPR_EXPECTED_TOKEN",
            DiagCode::ExprUnbalancedParen => "E_EXPR_UNBALANCED_PAREN",
            DiagCode::ExprTrailingInput => "E_EXPR_TRAILING_INPUT",
            // compat (warnings)
            DiagCode::CompatUndefinedValue => "W_COMPAT_UNDEFINED_VALUE",
            DiagCode::CompatLabelExtraSegmentDropped => "W_COMPAT_LABEL_EXTRA_SEGMENT_DROPPED",
            DiagCode::CompatCharaExtraSegmentDropped => "W_COMPAT_CHARA_EXTRA_SEGMENT_DROPPED",
            DiagCode::CompatLooseEndscript => "W_COMPAT_LOOSE_ENDSCRIPT",
            DiagCode::CompatCompensatedQuote => "W_COMPAT_COMPENSATED_QUOTE",
            // validation
            DiagCode::ValidDuplicateParam => "E_VALID_DUPLICATE_PARAM",
            DiagCode::ValidEmptyTagName => "E_VALID_EMPTY_TAG_NAME",
            // semantic
            DiagCode::SemUnknownLabel => "E_SEM_UNKNOWN_LABEL",
            DiagCode::SemDuplicateLabel => "E_SEM_DUPLICATE_LABEL",
        }
    }

    /// The severity this code carries unless overridden. `Compat*` codes are
    /// [`Severity::Warning`]; everything else is [`Severity::Error`].
    pub const fn default_severity(&self) -> Severity {
        match self {
            DiagCode::CompatUndefinedValue
            | DiagCode::CompatLabelExtraSegmentDropped
            | DiagCode::CompatCharaExtraSegmentDropped
            | DiagCode::CompatLooseEndscript
            | DiagCode::CompatCompensatedQuote => Severity::Warning,
            _ => Severity::Error,
        }
    }
}

// ======================================================================
// Secondary spans
// ======================================================================

/// The role a secondary span plays relative to the primary one.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum SecondaryKind {
    /// The construct was opened here (e.g. the unmatched `[`).
    OpenedHere,
    /// The first, winning definition lives here (e.g. the first label).
    FirstDefinedHere,
    /// Otherwise related context.
    RelatedHere,
}

// ======================================================================
// Diagnostic
// ======================================================================

/// A single structured diagnostic.
///
/// Build one with [`Diagnostic::new`] and refine it with the `with_*`
/// methods. The message text is not stored — call [`render`] to produce it.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Diagnostic {
    /// The stable code identifying this diagnostic.
    pub code: DiagCode,
    /// The severity (defaults from the code, but may be overridden).
    pub severity: Severity,
    /// The primary source span the diagnostic points at.
    pub primary: TextRange,
    /// Additional related spans and their roles.
    pub secondary: Vec<(TextRange, SecondaryKind)>,
    /// Syntax kinds that were expected (for `Expected*` codes).
    pub expected: Vec<SyntaxKind>,
    /// Named arguments interpolated into the rendered message.
    pub args: Vec<(&'static str, String)>,
}

impl Diagnostic {
    /// Starts a diagnostic for `code` at `primary`, taking the code's
    /// default severity.
    pub fn new(code: DiagCode, primary: TextRange) -> Diagnostic {
        Diagnostic {
            code,
            severity: code.default_severity(),
            primary,
            secondary: Vec::new(),
            expected: Vec::new(),
            args: Vec::new(),
        }
    }

    /// Overrides the severity (rarely needed; the code's default is usually
    /// right).
    #[must_use]
    pub fn with_severity(mut self, severity: Severity) -> Diagnostic {
        self.severity = severity;
        self
    }

    /// Attaches a related secondary span.
    #[must_use]
    pub fn with_secondary(mut self, range: TextRange, kind: SecondaryKind) -> Diagnostic {
        self.secondary.push((range, kind));
        self
    }

    /// Records the syntax kinds that were expected at the primary span.
    #[must_use]
    pub fn with_expected(mut self, expected: Vec<SyntaxKind>) -> Diagnostic {
        self.expected = expected;
        self
    }

    /// Adds a named message argument.
    #[must_use]
    pub fn with_arg(mut self, name: &'static str, value: impl Into<String>) -> Diagnostic {
        self.args.push((name, value.into()));
        self
    }

    /// Looks up a named argument added via [`Diagnostic::with_arg`].
    pub fn arg(&self, name: &str) -> Option<&str> {
        self.args
            .iter()
            .find(|(n, _)| *n == name)
            .map(|(_, v)| v.as_str())
    }
}

// ======================================================================
// Rendering
// ======================================================================

/// The languages [`render`] can produce messages in.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum Lang {
    /// English.
    En,
    /// Japanese.
    Ja,
}

/// Joins a diagnostic's `expected` kinds into a `` `a`, `b` `` list using
/// each kind's [`SyntaxKind::name`].
fn expected_list(d: &Diagnostic) -> String {
    d.expected
        .iter()
        .map(|k| format!("`{}`", k.name()))
        .collect::<Vec<_>>()
        .join(", ")
}

/// Renders `d` to a localized message string (no location prefix).
///
/// Every [`DiagCode`] has an English and a Japanese template; arguments and
/// the expected-kind list are interpolated in. Missing arguments render as
/// an empty string rather than panicking, so a partially-populated
/// diagnostic never crashes the renderer.
pub fn render(d: &Diagnostic, lang: Lang) -> String {
    let arg = |name: &str| d.arg(name).unwrap_or("");
    let expected = || expected_list(d);

    match (d.code, lang) {
        // -- lexical ---------------------------------------------------
        (DiagCode::LexUnterminatedString, Lang::En) => {
            "unterminated string; the closing quote is missing".to_owned()
        }
        (DiagCode::LexUnterminatedString, Lang::Ja) => {
            "文字列が閉じられていません。閉じ引用符がありません".to_owned()
        }
        (DiagCode::LexUnterminatedIscript, Lang::En) => {
            "`[iscript]` block is never closed; expected `[endscript]`".to_owned()
        }
        (DiagCode::LexUnterminatedIscript, Lang::Ja) => {
            "`[iscript]` ブロックが閉じられていません。`[endscript]` が必要です".to_owned()
        }
        (DiagCode::LexUnterminatedHtml, Lang::En) => {
            "`[html]` block is never closed; expected `[endhtml]`".to_owned()
        }
        (DiagCode::LexUnterminatedHtml, Lang::Ja) => {
            "`[html]` ブロックが閉じられていません。`[endhtml]` が必要です".to_owned()
        }
        (DiagCode::LexUnterminatedBlockComment, Lang::En) => {
            "block comment is never closed; expected `*/`".to_owned()
        }
        (DiagCode::LexUnterminatedBlockComment, Lang::Ja) => {
            "ブロックコメントが閉じられていません。`*/` が必要です".to_owned()
        }
        (DiagCode::LexStrayBackslashAtEol, Lang::En) => {
            "stray `\\` at end of line".to_owned()
        }
        (DiagCode::LexStrayBackslashAtEol, Lang::Ja) => {
            "行末に不要な `\\` があります".to_owned()
        }
        (DiagCode::LexInvalidBytes, Lang::En) => {
            "invalid character(s) in this context".to_owned()
        }
        (DiagCode::LexInvalidBytes, Lang::Ja) => {
            "この文脈では無効な文字です".to_owned()
        }

        // -- parse -----------------------------------------------------
        (DiagCode::ParseExpectedToken, Lang::En) => {
            format!("expected {}", expected())
        }
        (DiagCode::ParseExpectedToken, Lang::Ja) => {
            format!("{} が必要です", expected())
        }
        (DiagCode::ParseExpectedTagName, Lang::En) => "expected a tag name".to_owned(),
        (DiagCode::ParseExpectedTagName, Lang::Ja) => "タグ名が必要です".to_owned(),
        (DiagCode::ParseUnexpectedToken, Lang::En) => {
            format!("unexpected `{}`", arg("found"))
        }
        (DiagCode::ParseUnexpectedToken, Lang::Ja) => {
            format!("予期しない `{}` があります", arg("found"))
        }
        (DiagCode::ParseUnterminatedTag, Lang::En) => {
            "this tag is never closed; expected `]`".to_owned()
        }
        (DiagCode::ParseUnterminatedTag, Lang::Ja) => {
            "このタグが閉じられていません。`]` が必要です".to_owned()
        }

        // -- expression ------------------------------------------------
        (DiagCode::ExprExpectedOperand, Lang::En) => "expected an operand".to_owned(),
        (DiagCode::ExprExpectedOperand, Lang::Ja) => "オペランドが必要です".to_owned(),
        (DiagCode::ExprExpectedToken, Lang::En) => {
            format!("expected {}", expected())
        }
        (DiagCode::ExprExpectedToken, Lang::Ja) => {
            format!("{} が必要です", expected())
        }
        (DiagCode::ExprUnbalancedParen, Lang::En) => {
            "unbalanced parentheses in expression".to_owned()
        }
        (DiagCode::ExprUnbalancedParen, Lang::Ja) => {
            "式の括弧の対応が取れていません".to_owned()
        }
        (DiagCode::ExprTrailingInput, Lang::En) => {
            "unexpected trailing input after expression".to_owned()
        }
        (DiagCode::ExprTrailingInput, Lang::Ja) => {
            "式の後ろに余分な入力があります".to_owned()
        }

        // -- compat ----------------------------------------------------
        (DiagCode::CompatUndefinedValue, Lang::En) => {
            "`undefined` value is treated as an empty string, matching the engine".to_owned()
        }
        (DiagCode::CompatUndefinedValue, Lang::Ja) => {
            "`undefined` はエンジンと同様に空文字列として扱われます".to_owned()
        }
        (DiagCode::CompatLabelExtraSegmentDropped, Lang::En) => {
            "extra label segment is dropped, matching the engine".to_owned()
        }
        (DiagCode::CompatLabelExtraSegmentDropped, Lang::Ja) => {
            "余分なラベルセグメントはエンジンと同様に無視されます".to_owned()
        }
        (DiagCode::CompatCharaExtraSegmentDropped, Lang::En) => {
            "extra character-line segment is dropped, matching the engine".to_owned()
        }
        (DiagCode::CompatCharaExtraSegmentDropped, Lang::Ja) => {
            "余分なキャラクター行セグメントはエンジンと同様に無視されます".to_owned()
        }
        (DiagCode::CompatLooseEndscript, Lang::En) => {
            "block closed by a line merely containing `endscript`, matching the engine".to_owned()
        }
        (DiagCode::CompatLooseEndscript, Lang::Ja) => {
            "`endscript` を含むだけの行でブロックが閉じられました（エンジン互換）".to_owned()
        }
        (DiagCode::CompatCompensatedQuote, Lang::En) => {
            "missing closing quote was compensated, matching the engine".to_owned()
        }
        (DiagCode::CompatCompensatedQuote, Lang::Ja) => {
            "不足している閉じ引用符がエンジンと同様に補完されました".to_owned()
        }

        // -- validation ------------------------------------------------
        (DiagCode::ValidDuplicateParam, Lang::En) => {
            format!("duplicate parameter `{}`", arg("name"))
        }
        (DiagCode::ValidDuplicateParam, Lang::Ja) => {
            format!("パラメータ `{}` が重複しています", arg("name"))
        }
        (DiagCode::ValidEmptyTagName, Lang::En) => "tag name is empty".to_owned(),
        (DiagCode::ValidEmptyTagName, Lang::Ja) => "タグ名が空です".to_owned(),

        // -- semantic --------------------------------------------------
        (DiagCode::SemUnknownLabel, Lang::En) => {
            format!("unknown label `{}`", arg("name"))
        }
        (DiagCode::SemUnknownLabel, Lang::Ja) => {
            format!("未定義のラベル `{}` です", arg("name"))
        }
        (DiagCode::SemDuplicateLabel, Lang::En) => {
            format!("duplicate label `{}`", arg("name"))
        }
        (DiagCode::SemDuplicateLabel, Lang::Ja) => {
            format!("ラベル `{}` が重複しています", arg("name"))
        }
    }
}

/// Renders `d` prefixed with a 1-based `line:col: severity CODE:` location,
/// resolved against `source`.
///
/// The line and column are computed from the primary span's start via the
/// source's line index. Note that the column is a 1-based UTF-8 **byte**
/// column (the underlying [`crate::text::LineCol`] is 0-based byte-based;
/// this display adds 1 to each).
pub fn render_with_location(d: &Diagnostic, lang: Lang, source: &SourceText) -> String {
    let start: TextSize = d.primary.start();
    let lc = source.line_col(start);
    format!(
        "{}:{}: {} {}: {}",
        lc.line + 1,
        lc.col + 1,
        d.severity.label(),
        d.code.as_str(),
        render(d, lang)
    )
}

// ======================================================================
// Tests
// ======================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use crate::text::TextSize;

    fn tr(start: u32, end: u32) -> TextRange {
        TextRange::new(TextSize::new(start), TextSize::new(end))
    }

    #[test]
    fn default_severities() {
        assert_eq!(
            DiagCode::LexUnterminatedString.default_severity(),
            Severity::Error
        );
        assert_eq!(
            DiagCode::ParseUnterminatedTag.default_severity(),
            Severity::Error
        );
        assert_eq!(
            DiagCode::SemUnknownLabel.default_severity(),
            Severity::Error
        );
        // Every Compat* code is a warning.
        for code in [
            DiagCode::CompatUndefinedValue,
            DiagCode::CompatLabelExtraSegmentDropped,
            DiagCode::CompatCharaExtraSegmentDropped,
            DiagCode::CompatLooseEndscript,
            DiagCode::CompatCompensatedQuote,
        ] {
            assert_eq!(code.default_severity(), Severity::Warning, "{code:?}");
        }
    }

    #[test]
    fn as_str_stability() {
        assert_eq!(
            DiagCode::LexUnterminatedString.as_str(),
            "E_LEX_UNTERMINATED_STRING"
        );
        assert_eq!(
            DiagCode::ParseUnterminatedTag.as_str(),
            "E_PARSE_UNTERMINATED_TAG"
        );
        assert_eq!(
            DiagCode::CompatUndefinedValue.as_str(),
            "W_COMPAT_UNDEFINED_VALUE"
        );
        assert_eq!(DiagCode::SemDuplicateLabel.as_str(), "E_SEM_DUPLICATE_LABEL");
    }

    #[test]
    fn warning_codes_use_w_prefix() {
        // The prefix must agree with the default severity for every code.
        for code in [
            DiagCode::LexUnterminatedString,
            DiagCode::ParseExpectedToken,
            DiagCode::ExprUnbalancedParen,
            DiagCode::CompatUndefinedValue,
            DiagCode::CompatCompensatedQuote,
            DiagCode::ValidDuplicateParam,
            DiagCode::SemUnknownLabel,
        ] {
            let is_warning = code.default_severity() == Severity::Warning;
            let has_w = code.as_str().starts_with("W_");
            assert_eq!(is_warning, has_w, "{code:?} prefix/severity mismatch");
        }
    }

    #[test]
    fn builder_sets_default_severity() {
        let d = Diagnostic::new(DiagCode::CompatUndefinedValue, tr(0, 4));
        assert_eq!(d.severity, Severity::Warning);
        assert_eq!(d.primary, tr(0, 4));
        assert!(d.secondary.is_empty());
        assert!(d.expected.is_empty());
        assert!(d.args.is_empty());
    }

    #[test]
    fn builder_chains() {
        let d = Diagnostic::new(DiagCode::ParseUnterminatedTag, tr(10, 11))
            .with_secondary(tr(0, 1), SecondaryKind::OpenedHere)
            .with_expected(vec![SyntaxKind::R_BRACKET])
            .with_arg("found", "@")
            .with_severity(Severity::Warning);
        assert_eq!(d.severity, Severity::Warning);
        assert_eq!(d.secondary, vec![(tr(0, 1), SecondaryKind::OpenedHere)]);
        assert_eq!(d.expected, vec![SyntaxKind::R_BRACKET]);
        assert_eq!(d.arg("found"), Some("@"));
        assert_eq!(d.arg("missing"), None);
    }

    #[test]
    fn render_unterminated_tag_both_langs() {
        let d = Diagnostic::new(DiagCode::ParseUnterminatedTag, tr(0, 1));
        assert_eq!(render(&d, Lang::En), "this tag is never closed; expected `]`");
        assert_eq!(
            render(&d, Lang::Ja),
            "このタグが閉じられていません。`]` が必要です"
        );
    }

    #[test]
    fn render_expected_list_interpolation() {
        let d = Diagnostic::new(DiagCode::ParseExpectedToken, tr(0, 1))
            .with_expected(vec![SyntaxKind::EQ, SyntaxKind::R_BRACKET]);
        assert_eq!(render(&d, Lang::En), "expected `eq`, `r_bracket`");
        assert_eq!(render(&d, Lang::Ja), "`eq`, `r_bracket` が必要です");
    }

    #[test]
    fn render_arg_interpolation() {
        let d = Diagnostic::new(DiagCode::ValidDuplicateParam, tr(0, 3)).with_arg("name", "time");
        assert_eq!(render(&d, Lang::En), "duplicate parameter `time`");
        assert_eq!(render(&d, Lang::Ja), "パラメータ `time` が重複しています");
    }

    #[test]
    fn render_missing_arg_is_empty_not_panic() {
        let d = Diagnostic::new(DiagCode::SemUnknownLabel, tr(0, 1));
        assert_eq!(render(&d, Lang::En), "unknown label ``");
    }

    #[test]
    fn every_code_renders_in_both_langs() {
        // Guards against a missing match arm for any (code, lang) pair.
        let codes = [
            DiagCode::LexUnterminatedString,
            DiagCode::LexUnterminatedIscript,
            DiagCode::LexUnterminatedHtml,
            DiagCode::LexUnterminatedBlockComment,
            DiagCode::LexStrayBackslashAtEol,
            DiagCode::LexInvalidBytes,
            DiagCode::ParseExpectedToken,
            DiagCode::ParseExpectedTagName,
            DiagCode::ParseUnexpectedToken,
            DiagCode::ParseUnterminatedTag,
            DiagCode::ExprExpectedOperand,
            DiagCode::ExprExpectedToken,
            DiagCode::ExprUnbalancedParen,
            DiagCode::ExprTrailingInput,
            DiagCode::CompatUndefinedValue,
            DiagCode::CompatLabelExtraSegmentDropped,
            DiagCode::CompatCharaExtraSegmentDropped,
            DiagCode::CompatLooseEndscript,
            DiagCode::CompatCompensatedQuote,
            DiagCode::ValidDuplicateParam,
            DiagCode::ValidEmptyTagName,
            DiagCode::SemUnknownLabel,
            DiagCode::SemDuplicateLabel,
        ];
        for code in codes {
            let d = Diagnostic::new(code, tr(0, 1));
            assert!(!render(&d, Lang::En).is_empty(), "{code:?} EN empty");
            assert!(!render(&d, Lang::Ja).is_empty(), "{code:?} JA empty");
        }
    }

    #[test]
    fn render_with_location_multibyte() {
        // Line 0: "あ=1\n" (あ is 3 bytes). Line 1: "[tag".
        let source = SourceText::new("あ=1\n[tag");
        // Primary at the start of line 1 (byte 6).
        let d = Diagnostic::new(DiagCode::ParseUnterminatedTag, tr(6, 7))
            .with_secondary(tr(6, 7), SecondaryKind::OpenedHere);
        let rendered = render_with_location(&d, Lang::En, &source);
        assert_eq!(
            rendered,
            "2:1: error E_PARSE_UNTERMINATED_TAG: this tag is never closed; expected `]`"
        );
    }

    #[test]
    fn render_with_location_byte_column() {
        // "あ" is 3 bytes; a diagnostic at byte 3 sits at 1-based byte col 4.
        let source = SourceText::new("あx");
        let d = Diagnostic::new(DiagCode::ParseExpectedTagName, tr(3, 4));
        let rendered = render_with_location(&d, Lang::En, &source);
        assert_eq!(
            rendered,
            "1:4: error E_PARSE_EXPECTED_TAG_NAME: expected a tag name"
        );
    }

    #[test]
    fn diagnostic_is_clone_eq() {
        let d = Diagnostic::new(DiagCode::ValidEmptyTagName, tr(0, 0));
        assert_eq!(d.clone(), d);
    }
}
