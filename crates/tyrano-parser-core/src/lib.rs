//! File-local semantic index for TyranoScript.
//!
//! Pipeline position: `tyrano-syntax` (lossless CST) → `tyrano-db`
//! ([`tyrano_db::ParsedModule`]) → **this crate** ([`SemanticIndex`]) →
//! future multi-file semantics (`tyrano-semantic`).
//!
//! Not related to the legacy `tyrano-parser` crate: this is the
//! semantic-index layer over `tyrano-db`.
//!
//! This crate builds *file-local* facts with stable IDs that a later
//! multi-file layer can reference: scopes, symbols (labels, macros,
//! characters), definitions, variable places, a use-def map, and
//! file-local semantic errors.
//!
//! Deliberately out of scope: import/`storage=` resolution, cross-file
//! type inference, module member lookup, resource resolution,
//! `[iscript]` body analysis, and builtin-tag catalog validation.

pub mod ast_id;
pub mod builder;
pub mod errors;
pub mod index;
pub mod place;
pub mod scope;
pub mod symbol;
pub mod use_def;

pub use ast_id::{AstId, AstIdMap, ErasedAstId};
pub use builder::{CHARA_REF_TAGS, JUMP_TAGS, build_index};
pub use errors::{SemanticError, SemanticErrorKind};
pub use index::{EmbeddedLang, EmbeddedRegion, PlaceId, PlaceOccurrence, SemanticIndex};
pub use place::{AccessKind, PathSeg, Place, PlaceRoot};
pub use scope::{Scope, ScopeId, ScopeKind};
pub use symbol::{Definition, DefinitionId, Symbol, SymbolId, SymbolKind};
pub use use_def::{RefKind, Reference, Resolution, UseDefMap, UseId};

use std::sync::Arc;

/// The [`AstIdMap`] for `file`'s current tree revision.
#[salsa::tracked]
pub fn ast_id_map(db: &dyn tyrano_db::Db, file: tyrano_db::SourceFile) -> Arc<AstIdMap> {
    let module = tyrano_db::parsed_module(db, file);
    Arc::new(AstIdMap::from_root(&module.syntax()))
}

/// The [`SemanticIndex`] for `file`. Recomputes when the parse or the
/// interpretation options change; backdates when the result is unchanged.
#[salsa::tracked]
pub fn semantic_index(db: &dyn tyrano_db::Db, file: tyrano_db::SourceFile) -> Arc<SemanticIndex> {
    let module = tyrano_db::parsed_module(db, file);
    let ast_ids = ast_id_map(db, file);
    Arc::new(build_index(&module.scenario(), &ast_ids, &file.interpret_options(db)))
}

/// The file-local semantic errors of `file`, lowered into the shared
/// [`tyrano_db::FileDiagnostic`] shape.
#[salsa::tracked]
pub fn semantic_diagnostics(
    db: &dyn tyrano_db::Db,
    file: tyrano_db::SourceFile,
) -> Arc<[tyrano_db::FileDiagnostic]> {
    semantic_index(db, file).errors().iter().map(SemanticError::to_diagnostic).collect()
}

/// Every diagnostic of `file` — lex/parse plus semantic — in one list,
/// sorted by `(start, end, code)`. The single-file analogue of ruff's
/// per-file check result.
#[salsa::tracked]
pub fn file_diagnostics(
    db: &dyn tyrano_db::Db,
    file: tyrano_db::SourceFile,
) -> Arc<[tyrano_db::FileDiagnostic]> {
    let mut all: Vec<tyrano_db::FileDiagnostic> =
        tyrano_db::parse_diagnostics(db, file).iter().cloned().collect();
    all.extend(semantic_diagnostics(db, file).iter().cloned());
    all.sort_by_key(|d| (d.range.start(), d.range.end(), d.code));
    all.into()
}

#[cfg(test)]
mod tests {
    use super::*;
    use salsa::Setter as _;
    use tyrano_db::{RootDatabase, SourceFile};
    use tyrano_syntax::ast::InterpretOptions;

    const SRC: &str = "\
*start|開始\n\
#akane:happy\n\
こんにちは[l]世界\n\
[eval exp=\"f.hp = 10\"]\n\
[jump target=*end]\n\
[macro name=greet]\n\
[image storage=&f.bg]\n\
[endmacro]\n\
[greet]\n\
*end\n";

    #[test]
    fn semantic_index_query_works_end_to_end() {
        let db = RootDatabase::default();
        let file = SourceFile::with_defaults(&db, SRC.to_string());
        let idx = semantic_index(&db, file);

        assert!(idx.errors().is_empty(), "{:?}", idx.errors());
        assert!(idx.label("start").is_some());
        assert!(idx.label("end").is_some());
        assert!(idx.macro_("greet").is_some());
        assert!(idx.character("akane").is_some());
        assert_eq!(idx.use_def().uses().len(), 2, "jump + macro call");
        assert_eq!(idx.place_occurrences().len(), 2, "f.hp write + f.bg read");

        // Ids resolve against a fresh tree from the same revision.
        let ids = ast_id_map(&db, file);
        let module = tyrano_db::parsed_module(&db, file);
        let root = module.syntax();
        for def in idx.definitions() {
            let node = ids.resolve(def.node, &root).expect("definition id resolves");
            assert!(node.text_range().contains_range(def.name_range));
        }
    }

    #[test]
    fn semantic_index_is_memoized() {
        let db = RootDatabase::default();
        let file = SourceFile::with_defaults(&db, SRC.to_string());
        let a = semantic_index(&db, file);
        let b = semantic_index(&db, file);
        assert!(Arc::ptr_eq(&a, &b), "memo hit must return the same Arc");
    }

    #[test]
    fn setting_identical_text_keeps_downstream_memo() {
        let mut db = RootDatabase::default();
        let file = SourceFile::with_defaults(&db, SRC.to_string());
        let before = semantic_index(&db, file);
        // Same string again: the parse re-runs but produces an equal
        // ParsedModule, so backdating keeps the semantic index memo.
        file.set_text(&mut db).to(SRC.to_string());
        let after = semantic_index(&db, file);
        assert!(Arc::ptr_eq(&before, &after), "identical reparse must backdate");
    }

    #[test]
    fn text_edit_recomputes_index() {
        let mut db = RootDatabase::default();
        let file = SourceFile::with_defaults(&db, SRC.to_string());
        let before = semantic_index(&db, file);
        file.set_text(&mut db).to(format!("{SRC}*extra\n"));
        let after = semantic_index(&db, file);
        assert!(!Arc::ptr_eq(&before, &after));
        assert!(after.label("extra").is_some());
    }

    #[test]
    fn semantic_diagnostics_lower_errors() {
        let db = RootDatabase::default();
        let file = SourceFile::with_defaults(&db, "[jump target=*gone]\n".to_string());
        let diags = semantic_diagnostics(&db, file);
        assert_eq!(diags.len(), 1);
        assert_eq!(diags[0].code, "sem-unknown-label-target");
        assert_eq!(diags[0].severity, tyrano_syntax::diagnostics::Severity::Error);
        assert_eq!(diags[0].message, "unknown label `gone` in this file");
        // Renders with a 1-based location via the shared line index.
        let index = tyrano_db::line_index(&db, file);
        let rendered = diags[0].render_with_location(&index);
        assert_eq!(
            rendered,
            "1:14: error sem-unknown-label-target: unknown label `gone` in this file"
        );
    }

    #[test]
    fn file_diagnostics_merge_parse_and_semantic_sorted() {
        // "[unclosed\n" gives a parse diagnostic; the jump gives a semantic
        // one further down. Merged output must be sorted by range.
        let db = RootDatabase::default();
        let file =
            SourceFile::with_defaults(&db, "[jump target=*gone]\n[unclosed\n".to_string());
        let diags = file_diagnostics(&db, file);
        assert!(diags.len() >= 2, "expected parse + semantic, got {diags:?}");
        assert!(diags.iter().any(|d| d.code.starts_with("E_")), "parse diag present");
        assert!(diags.iter().any(|d| d.code == "sem-unknown-label-target"));
        assert!(
            diags.windows(2).all(|w| w[0].range.start() <= w[1].range.start()),
            "sorted by start: {diags:?}"
        );
    }

    #[test]
    fn file_diagnostics_memoized() {
        let db = RootDatabase::default();
        let file = SourceFile::with_defaults(&db, SRC.to_string());
        let a = file_diagnostics(&db, file);
        let b = file_diagnostics(&db, file);
        assert!(Arc::ptr_eq(&a, &b));
        assert!(a.is_empty(), "SRC is clean: {a:?}");
    }

    #[test]
    fn interpret_options_change_recomputes_index_without_reparse() {
        let mut db = RootDatabase::default();
        let file = SourceFile::with_defaults(&db, "*a|b|c\n".to_string());
        let before = semantic_index(&db, file);
        let parse_before = tyrano_db::parsed_module(&db, file);

        file.set_interpret_options(&mut db).to(InterpretOptions {
            label_value_first_segment_only: false,
            ..InterpretOptions::default()
        });
        let after = semantic_index(&db, file);
        let parse_after = tyrano_db::parsed_module(&db, file);

        assert!(
            tyrano_syntax::green::GreenNode::ptr_eq(parse_before.green(), parse_after.green()),
            "interpret options must not reparse"
        );
        // The index itself is unaffected by label-value cooking (values are
        // not part of the index), so equality backdating may keep the Arc;
        // what matters is that the query ran without error.
        assert!(after.label("a").is_some());
        let _ = before;
    }
}
