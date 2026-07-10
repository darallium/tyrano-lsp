//! Layer-agnostic, rendered diagnostics for a file.
//!
//! Adapted from `ruff_db::diagnostic` (minimal). Syntax diagnostics
//! ([`tyrano_syntax`]) and, later, semantic errors ([`tyrano-parser-core`])
//! both lower into a single [`FileDiagnostic`] shape so consumers get one
//! flat list with stable codes, ranges, and already-rendered messages.

use std::sync::Arc;

use tyrano_syntax::diagnostics::{Lang, Severity, render};
use tyrano_syntax::text::{LineIndex, TextRange};

/// One rendered, layer-agnostic diagnostic for a file.
///
/// Both syntax diagnostics ([`tyrano_syntax`]) and semantic errors
/// ([`tyrano-parser-core`]) lower into this shape so consumers get a single
/// list.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileDiagnostic {
    /// Stable machine code (`"E_PARSE_…"`, `"E_SEM_…"`, …).
    pub code: &'static str,
    /// Severity, reusing [`tyrano_syntax::diagnostics::Severity`].
    pub severity: Severity,
    /// The primary source span.
    pub range: TextRange,
    /// Human-readable message (already language-rendered).
    pub message: String,
}

impl FileDiagnostic {
    /// Lowers a structured syntax [`Diagnostic`](tyrano_syntax::diagnostics::Diagnostic)
    /// into a flat [`FileDiagnostic`], rendering its message in `lang`.
    pub fn from_syntax(
        d: &tyrano_syntax::diagnostics::Diagnostic,
        lang: Lang,
    ) -> FileDiagnostic {
        FileDiagnostic {
            code: d.code.as_str(),
            severity: d.severity,
            range: d.primary,
            message: render(d, lang),
        }
    }

    /// Renders `"{line+1}:{col+1}: {severity label} {code}: {message}"` —
    /// the same shape as
    /// [`tyrano_syntax::diagnostics::render_with_location`].
    ///
    /// Line and column are 1-based; the column is a UTF-8 **byte** column
    /// (the underlying [`LineCol`](tyrano_syntax::text::LineCol) is 0-based,
    /// this display adds 1).
    pub fn render_with_location(&self, index: &LineIndex) -> String {
        let lc = index.line_col(self.range.start());
        format!(
            "{}:{}: {} {}: {}",
            lc.line + 1,
            lc.col + 1,
            self.severity.label(),
            self.code,
            self.message
        )
    }
}

/// All lex/parse diagnostics of `file` as [`FileDiagnostic`]s (English
/// messages).
///
/// For other languages use `ParsedModule::diagnostics()` plus
/// [`FileDiagnostic::from_syntax`] at display time. Sorted by primary span,
/// inheriting the ordering that `tyrano-syntax` already guarantees.
#[salsa::tracked]
pub fn parse_diagnostics(db: &dyn crate::Db, file: crate::SourceFile) -> Arc<[FileDiagnostic]> {
    let module = crate::parsed_module(db, file);
    module
        .diagnostics()
        .iter()
        .map(|d| FileDiagnostic::from_syntax(d, Lang::En))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{RootDatabase, SourceFile};
    use tyrano_syntax::parse;
    use tyrano_syntax::text::LineIndex;

    #[test]
    fn from_syntax_maps_fields() {
        let parse = parse("[unclosed\n");
        let d = parse
            .diagnostics()
            .first()
            .expect("broken input yields a diagnostic");
        let fd = FileDiagnostic::from_syntax(d, Lang::En);
        assert!(fd.code.starts_with("E_"), "code was {}", fd.code);
        assert_eq!(fd.severity, Severity::Error);
        assert!(!fd.message.is_empty());
        assert_eq!(fd.range, d.primary);
    }

    #[test]
    fn render_with_location_format() {
        let text = "*a\n[x\n";
        let parse = parse(text);
        let d = parse
            .diagnostics()
            .first()
            .expect("broken input yields a diagnostic");
        let fd = FileDiagnostic::from_syntax(d, Lang::En);
        let index = LineIndex::new(text);
        let lc = index.line_col(fd.range.start());
        let expected = format!(
            "{}:{}: {} {}: {}",
            lc.line + 1,
            lc.col + 1,
            fd.severity.label(),
            fd.code,
            fd.message
        );
        assert_eq!(fd.render_with_location(&index), expected);
        // Location must be 1-based on both axes.
        assert!(expected.split(':').next().unwrap().parse::<u32>().unwrap() >= 1);
    }

    #[test]
    fn parse_diagnostics_query() {
        let db = RootDatabase::default();
        let clean = SourceFile::with_defaults(&db, "*start\n".to_string());
        assert!(parse_diagnostics(&db, clean).is_empty());

        let broken = SourceFile::with_defaults(&db, "[unclosed\n[also\n".to_string());
        let diags = parse_diagnostics(&db, broken);
        assert!(!diags.is_empty());
        // Sorted by range start (tyrano-syntax already sorts; assert anyway).
        assert!(
            diags.windows(2).all(|w| w[0].range.start() <= w[1].range.start()),
            "diagnostics must be sorted by range start"
        );
    }

    #[test]
    fn parse_diagnostics_memoized() {
        let db = RootDatabase::default();
        let file = SourceFile::with_defaults(&db, "[unclosed\n".to_string());
        let a = parse_diagnostics(&db, file);
        let b = parse_diagnostics(&db, file);
        assert!(Arc::ptr_eq(&a, &b), "memo hit must return the same Arc");
    }
}
