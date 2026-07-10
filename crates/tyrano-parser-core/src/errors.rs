//! File-local semantic errors for the TyranoScript semantic index.
//!
//! This crate builds its own file-local facts over `tyrano-db`'s parsed
//! module and needs a semantic error vocabulary distinct from
//! `tyrano_syntax::diagnostics::DiagCode`: labels, macros, and jump
//! targets are concepts this layer owns, not the lossless syntax layer.
//! [`SemanticError`] is therefore a small, self-contained type — it does
//! not wrap or extend `DiagCode`.

use tyrano_syntax::diagnostics::Severity;
use tyrano_syntax::text::TextRange;

/// The kind of a file-local semantic error, with the data needed to
/// render a message and (for duplicates) point at the earlier site.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SemanticErrorKind {
    /// A label name defined more than once; `first` is the winning (first) site.
    DuplicateLabel { name: String, first: TextRange },
    /// `[jump target=*x]` (or call/link/button) with no `storage=` and no
    /// such label in this file.
    UnknownLabelTarget { name: String },
    /// `[macro]` without a `name=` parameter (or with an empty name).
    MacroMissingName,
    /// `[macro]` reached end of file without `[endmacro]`.
    UnclosedMacro { name: String },
    /// `[endmacro]` with no open `[macro]`.
    StrayEndMacro,
    /// `[macro]` while a macro body is already open (the engine does not
    /// nest macros).
    NestedMacro { outer: String },
    /// A macro name defined more than once; `first` is the winning (first) site.
    DuplicateMacro { name: String, first: TextRange },
    /// The same parameter name given more than once on one tag. The engine
    /// fills a JS object left to right, so the last value silently wins —
    /// almost certainly a script bug, but the tag still runs (warning).
    DuplicateParam { name: String },
}

/// A single file-local semantic error: a [`SemanticErrorKind`] anchored at
/// a primary [`TextRange`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SemanticError {
    pub kind: SemanticErrorKind,
    /// Primary range (label name token, tag range, ...).
    pub range: TextRange,
}

impl SemanticError {
    /// Builds a [`SemanticError`] from its kind and primary range.
    pub fn new(kind: SemanticErrorKind, range: TextRange) -> SemanticError {
        SemanticError { kind, range }
    }

    /// Stable machine-readable code, kebab-case with a `sem-` prefix.
    pub fn code(&self) -> &'static str {
        match &self.kind {
            SemanticErrorKind::DuplicateLabel { .. } => "sem-duplicate-label",
            SemanticErrorKind::UnknownLabelTarget { .. } => "sem-unknown-label-target",
            SemanticErrorKind::MacroMissingName => "sem-macro-missing-name",
            SemanticErrorKind::UnclosedMacro { .. } => "sem-unclosed-macro",
            SemanticErrorKind::StrayEndMacro => "sem-stray-endmacro",
            SemanticErrorKind::NestedMacro { .. } => "sem-nested-macro",
            SemanticErrorKind::DuplicateMacro { .. } => "sem-duplicate-macro",
            SemanticErrorKind::DuplicateParam { .. } => "sem-duplicate-param",
        }
    }

    /// How severe this error is. Everything that breaks resolution or
    /// definition structure is an [`Severity::Error`]; conditions the
    /// engine tolerates at runtime are [`Severity::Warning`].
    pub fn severity(&self) -> Severity {
        match &self.kind {
            SemanticErrorKind::DuplicateParam { .. } => Severity::Warning,
            _ => Severity::Error,
        }
    }

    /// Human-readable English message including the relevant name in
    /// backticks. One line, lowercase start, no trailing period.
    pub fn message(&self) -> String {
        match &self.kind {
            SemanticErrorKind::DuplicateLabel { name, .. } => {
                format!("duplicate label `{name}` (first defined earlier)")
            }
            SemanticErrorKind::UnknownLabelTarget { name } => {
                format!("unknown label `{name}` in this file")
            }
            SemanticErrorKind::MacroMissingName => "macro is missing a name= parameter".to_owned(),
            SemanticErrorKind::UnclosedMacro { name } => {
                format!("macro `{name}` is never closed with [endmacro]")
            }
            SemanticErrorKind::StrayEndMacro => "[endmacro] with no matching [macro]".to_owned(),
            SemanticErrorKind::NestedMacro { outer } => {
                format!("macro nested inside macro `{outer}` (macros do not nest)")
            }
            SemanticErrorKind::DuplicateMacro { name, .. } => {
                format!("duplicate macro `{name}` (first defined earlier)")
            }
            SemanticErrorKind::DuplicateParam { name } => {
                format!("duplicate parameter `{name}` on this tag (the last value wins)")
            }
        }
    }

    /// Lowers this error into the layer-agnostic diagnostic shape shared
    /// with syntax diagnostics (see [`tyrano_db::FileDiagnostic`]).
    pub fn to_diagnostic(&self) -> tyrano_db::FileDiagnostic {
        tyrano_db::FileDiagnostic {
            code: self.code(),
            severity: self.severity(),
            range: self.range,
            message: self.message(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tyrano_syntax::text::TextSize;

    fn tr(a: u32, b: u32) -> TextRange {
        TextRange::new(TextSize::new(a), TextSize::new(b))
    }

    fn all_kinds() -> Vec<SemanticErrorKind> {
        vec![
            SemanticErrorKind::DuplicateLabel { name: "start".to_owned(), first: tr(0, 5) },
            SemanticErrorKind::UnknownLabelTarget { name: "gone".to_owned() },
            SemanticErrorKind::MacroMissingName,
            SemanticErrorKind::UnclosedMacro { name: "greet".to_owned() },
            SemanticErrorKind::StrayEndMacro,
            SemanticErrorKind::NestedMacro { outer: "outer".to_owned() },
            SemanticErrorKind::DuplicateMacro { name: "greet".to_owned(), first: tr(0, 5) },
            SemanticErrorKind::DuplicateParam { name: "storage".to_owned() },
        ]
    }

    #[test]
    fn codes_are_stable() {
        let expected = [
            "sem-duplicate-label",
            "sem-unknown-label-target",
            "sem-macro-missing-name",
            "sem-unclosed-macro",
            "sem-stray-endmacro",
            "sem-nested-macro",
            "sem-duplicate-macro",
            "sem-duplicate-param",
        ];
        let errors: Vec<SemanticError> =
            all_kinds().into_iter().map(|k| SemanticError::new(k, tr(10, 20))).collect();
        let codes: Vec<&str> = errors.iter().map(SemanticError::code).collect();
        assert_eq!(codes, expected);
    }

    #[test]
    fn messages_include_names() {
        let err = |kind| SemanticError::new(kind, tr(10, 20));

        assert_eq!(
            err(SemanticErrorKind::DuplicateLabel { name: "start".to_owned(), first: tr(0, 5) })
                .message(),
            "duplicate label `start` (first defined earlier)"
        );
        assert_eq!(
            err(SemanticErrorKind::UnknownLabelTarget { name: "gone".to_owned() }).message(),
            "unknown label `gone` in this file"
        );
        assert_eq!(
            err(SemanticErrorKind::MacroMissingName).message(),
            "macro is missing a name= parameter"
        );
        assert_eq!(
            err(SemanticErrorKind::UnclosedMacro { name: "greet".to_owned() }).message(),
            "macro `greet` is never closed with [endmacro]"
        );
        assert_eq!(
            err(SemanticErrorKind::StrayEndMacro).message(),
            "[endmacro] with no matching [macro]"
        );
        assert_eq!(
            err(SemanticErrorKind::NestedMacro { outer: "outer".to_owned() }).message(),
            "macro nested inside macro `outer` (macros do not nest)"
        );
        assert_eq!(
            err(SemanticErrorKind::DuplicateMacro { name: "greet".to_owned(), first: tr(0, 5) })
                .message(),
            "duplicate macro `greet` (first defined earlier)"
        );
        assert_eq!(
            err(SemanticErrorKind::DuplicateParam { name: "storage".to_owned() }).message(),
            "duplicate parameter `storage` on this tag (the last value wins)"
        );
    }

    #[test]
    fn severities() {
        for kind in all_kinds() {
            let err = SemanticError::new(kind.clone(), tr(0, 1));
            let expected = match kind {
                SemanticErrorKind::DuplicateParam { .. } => Severity::Warning,
                _ => Severity::Error,
            };
            assert_eq!(err.severity(), expected, "{:?}", err.kind);
        }
    }

    #[test]
    fn errors_are_comparable() {
        let a = SemanticError::new(
            SemanticErrorKind::UnknownLabelTarget { name: "gone".to_owned() },
            tr(10, 20),
        );
        let b = a.clone();
        assert_eq!(a, b);

        let c = SemanticError::new(SemanticErrorKind::StrayEndMacro, tr(10, 20));
        assert_ne!(a, c);

        // Same kind, different range: not equal.
        let d = SemanticError::new(
            SemanticErrorKind::UnknownLabelTarget { name: "gone".to_owned() },
            tr(0, 1),
        );
        assert_ne!(a, d);
    }
}
