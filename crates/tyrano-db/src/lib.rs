//! Incremental (salsa-backed) parse database for TyranoScript.
//!
//! Pipeline position: source text → `tyrano-syntax` (lossless CST) →
//! **this crate** ([`ParsedModule`]) → `tyrano-parser-core`
//! (file-local semantic index).
//!
//! This layer is deliberately thin: text in, [`ParsedModule`] out. It knows
//! nothing about the file system, projects, or search paths — callers
//! (tests, the CLI, a future project layer) create [`SourceFile`] inputs
//! and mutate their text. Downstream crates define their own
//! `#[salsa::tracked]` functions over [`Db`], so this crate stays the only
//! place that owns database plumbing.

pub mod diagnostic;
mod parsed;
pub mod source;

pub use diagnostic::{FileDiagnostic, parse_diagnostics};
pub use parsed::ParsedModule;
pub use source::line_index;

use tyrano_syntax::ParseOptions;
use tyrano_syntax::ast::InterpretOptions;

/// One in-memory `.ks` source.
///
/// `interpret_options` is a separate field from `parse_options` so that
/// toggling engine-interpretation quirks invalidates only downstream
/// semantic queries, never the parse itself.
#[salsa::input]
pub struct SourceFile {
    /// UTF-8 source text.
    #[returns(ref)]
    pub text: String,
    /// Tree-shape options consumed by [`parsed_module`].
    pub parse_options: ParseOptions,
    /// Engine-quirk interpretation options, consumed downstream
    /// (`tyrano-parser-core`); threaded through the db as an input.
    pub interpret_options: InterpretOptions,
}

impl SourceFile {
    /// A file with default parse and interpretation options.
    pub fn with_defaults(db: &dyn Db, text: String) -> SourceFile {
        SourceFile::new(db, text, ParseOptions::default(), InterpretOptions::default())
    }
}

/// The database trait the whole pipeline is written against; downstream
/// crates take `&dyn tyrano_db::Db`.
#[salsa::db]
pub trait Db: salsa::Database {}

/// Concrete database for tests, the CLI, and (later) the LSP session.
#[salsa::db]
#[derive(Default, Clone)]
pub struct RootDatabase {
    storage: salsa::Storage<Self>,
}

#[salsa::db]
impl salsa::Database for RootDatabase {}

#[salsa::db]
impl Db for RootDatabase {}

/// Parses `file` (never fails; broken input yields ERROR nodes plus
/// diagnostics). Memoized: re-runs only when `text` or `parse_options`
/// change, and backdates when the reparse is structurally identical.
#[salsa::tracked]
pub fn parsed_module(db: &dyn Db, file: SourceFile) -> ParsedModule {
    let parse = tyrano_syntax::parse_with_options(file.text(db), &file.parse_options(db));
    ParsedModule::from_parse(&parse)
}

#[cfg(test)]
mod tests {
    use super::*;
    use salsa::Setter as _;
    use tyrano_syntax::green::GreenNode;

    const SRC: &str = "*start\nこんにちは\n[jump target=*start]\n";

    #[test]
    fn query_parses_source() {
        let db = RootDatabase::default();
        let file = SourceFile::with_defaults(&db, SRC.to_string());
        let module = parsed_module(&db, file);
        assert_eq!(module.green().to_source(), SRC);
        assert!(module.diagnostics().is_empty());
    }

    #[test]
    fn query_is_memoized() {
        let db = RootDatabase::default();
        let file = SourceFile::with_defaults(&db, SRC.to_string());
        let a = parsed_module(&db, file);
        let b = parsed_module(&db, file);
        assert!(GreenNode::ptr_eq(a.green(), b.green()), "memo hit must return the same tree");
    }

    #[test]
    fn query_invalidates_on_text_change() {
        let mut db = RootDatabase::default();
        let file = SourceFile::with_defaults(&db, SRC.to_string());
        let before = parsed_module(&db, file);
        file.set_text(&mut db).to("*other\n".to_string());
        let after = parsed_module(&db, file);
        assert_ne!(before, after);
        assert_eq!(after.green().to_source(), "*other\n");
    }

    #[test]
    fn interpret_options_change_does_not_reparse() {
        let mut db = RootDatabase::default();
        let file = SourceFile::with_defaults(&db, SRC.to_string());
        let before = parsed_module(&db, file);
        file.set_interpret_options(&mut db).to(InterpretOptions {
            label_value_first_segment_only: false,
            ..InterpretOptions::default()
        });
        let after = parsed_module(&db, file);
        assert!(
            GreenNode::ptr_eq(before.green(), after.green()),
            "parse must not depend on interpret_options"
        );
    }

    #[test]
    fn parse_options_change_reparses() {
        let mut db = RootDatabase::default();
        let file = SourceFile::with_defaults(&db, SRC.to_string());
        let before = parsed_module(&db, file);
        file.set_parse_options(&mut db)
            .to(ParseOptions { loose_endscript_termination: false });
        let after = parsed_module(&db, file);
        assert!(!GreenNode::ptr_eq(before.green(), after.green()), "options are a parse input");
        assert_eq!(after.options().loose_endscript_termination, false);
    }
}
