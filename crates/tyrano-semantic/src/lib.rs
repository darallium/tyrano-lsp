//! Cross-file semantics for TyranoScript projects.
//!
//! Pipeline position: `tyrano-parser-core` (file-local index) +
//! `tyrano-project` (files, project inputs) + `tyrano-module-resolver`
//! (`storage=` resolution) → **this crate**: label export sets, the
//! dependency graph, and the [`SemanticModel`] facade an LSP layer talks
//! to.
//!
//! No new database trait: every query takes `&dyn ProjectDb` — this layer
//! adds queries, not plumbing.

pub mod check;
pub mod codes;
mod deps;
mod labels;
mod model;

pub use deps::{DependencyGraph, file_dependencies, project_dependency_graph};
pub use labels::exported_labels;
pub use model::{
    LabelResolution, LabelTarget, ProjectResolution, SemanticModel, TagResolution,
};

use std::sync::Arc;

use tyrano_db::FileDiagnostic;
use tyrano_project::{File, ProjectDb};

/// Every diagnostic of `file` at project scope: the file-local list from
/// `tyrano-parser-core` (lex/parse + `sem-*`) merged with this crate's
/// tag and cross-file checks (`xsem-*`), sorted by `(start, end, code)`.
#[salsa::tracked]
pub fn check_file(db: &dyn ProjectDb, file: File) -> Arc<[FileDiagnostic]> {
    let mut all: Vec<FileDiagnostic> =
        tyrano_parser_core::file_diagnostics(db, file.source(db)).iter().cloned().collect();
    all.extend(check::tags::check_tags(db, file));
    all.extend(check::xfile::check_xfile(db, file));
    all.sort_by_key(|d| (d.range.start(), d.range.end(), d.code));
    all.into()
}

/// [`check_file`] over every scenario file, in path order. A plain
/// function: the per-file queries are the memoization boundary.
pub fn check_project(db: &dyn ProjectDb) -> Vec<(File, Arc<[FileDiagnostic]>)> {
    db.project()
        .scenario_files(db)
        .iter()
        .map(|&file| (file, check_file(db, file)))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use tyrano_project::macros::MacroDef;
    use tyrano_project::testing::ProjectBuilder;
    use tyrano_project::{AssetKind, ProjectDatabase, ProjectPath};

    fn file(db: &ProjectDatabase, path: &str) -> File {
        db.file(&ProjectPath::new(path).unwrap()).expect("fixture file exists")
    }

    fn two_file_project() -> ProjectDatabase {
        ProjectBuilder::new()
            .file(
                "data/scenario/first.ks",
                "*start\n[jump storage=scene2.ks target=*top]\n[jump target=*start]\n",
            )
            .file("data/scenario/scene2.ks", "*top\n*bottom\ntext\n")
            .build()
    }

    #[test]
    fn exported_labels_lists_label_names() {
        let db = two_file_project();
        let labels = exported_labels(&db, file(&db, "data/scenario/scene2.ks"));
        let names: Vec<&str> = labels.iter().map(String::as_str).collect();
        assert_eq!(names, ["bottom", "top"]);
    }

    #[test]
    fn dependencies_and_graph() {
        let db = two_file_project();
        let first = file(&db, "data/scenario/first.ks");
        let scene2 = file(&db, "data/scenario/scene2.ks");

        let deps = file_dependencies(&db, first);
        assert_eq!(&*deps, &[scene2]);
        assert!(file_dependencies(&db, scene2).is_empty());

        let graph = project_dependency_graph(&db, db.project());
        assert_eq!(graph.dependencies(first), &[scene2]);
        assert_eq!(graph.dependents(scene2), &[first]);
        assert_eq!(graph.dependents(first), &[] as &[File]);

        let model = SemanticModel::new(&db, first);
        assert_eq!(&*model.dependencies(), &[scene2]);
        assert_eq!(SemanticModel::new(&db, scene2).dependents(), vec![first]);
    }

    #[test]
    fn resolve_label_local_and_cross_file() {
        let db = two_file_project();
        let first = file(&db, "data/scenario/first.ks");
        let scene2 = file(&db, "data/scenario/scene2.ks");
        let model = SemanticModel::new(&db, first);

        let local = model.resolve_label("start").expect("local label");
        assert_eq!(local.file, first);
        assert_eq!(model.resolve_label("top"), None, "other file's label is not local");

        match model.resolve_label_in("scene2.ks", "top") {
            LabelResolution::Found(t) => assert_eq!(t.file, scene2),
            other => panic!("expected Found, got {other:?}"),
        }
        assert_eq!(
            model.resolve_label_in("scene2.ks", "gone"),
            LabelResolution::LabelNotFound(scene2)
        );
        assert_eq!(model.resolve_label_in("nowhere.ks", "top"), LabelResolution::FileNotFound);
    }

    #[test]
    fn resolve_reference_lifts_external_uses() {
        let db = two_file_project();
        let first = file(&db, "data/scenario/first.ks");
        let scene2 = file(&db, "data/scenario/scene2.ks");
        let model = SemanticModel::new(&db, first);

        // Document order: [0] external jump, [1] local jump.
        let resolved = model.resolved_references();
        assert_eq!(resolved.len(), 2);
        match &resolved[0] {
            ProjectResolution::ExternalLabel(t) => assert_eq!(t.file, scene2),
            other => panic!("expected ExternalLabel, got {other:?}"),
        }
        match &resolved[1] {
            ProjectResolution::Local(def) => {
                // The UseId-based form agrees with the enumeration.
                let index = model.index();
                let uses = index.use_def().uses_of(*def);
                assert_eq!(model.resolve_reference(uses[0]), resolved[1]);
            }
            other => panic!("expected Local, got {other:?}"),
        }
    }

    #[test]
    fn resolve_reference_reports_missing_targets() {
        let db = ProjectBuilder::new()
            .file(
                "data/scenario/first.ks",
                "[jump storage=gone.ks target=*x]\n[jump storage=scene2.ks target=*missing]\n[jump storage=scene2.ks]\n",
            )
            .file("data/scenario/scene2.ks", "*top\n")
            .build();
        let model = SemanticModel::new(&db, file(&db, "data/scenario/first.ks"));
        let scene2 = file(&db, "data/scenario/scene2.ks");

        let resolved = model.resolved_references();
        assert_eq!(
            resolved,
            vec![
                ProjectResolution::FileNotFound { storage: "gone.ks".to_string() },
                ProjectResolution::LabelNotFound { file: scene2, label: "missing".to_string() },
                ProjectResolution::ExternalFile(scene2),
            ]
        );
    }

    #[test]
    fn resolve_macro_prefers_local_then_project() {
        let db = ProjectBuilder::new()
            .file("data/scenario/a.ks", "[macro name=greet]a[endmacro]\n")
            .file("data/scenario/b.ks", "[macro name=greet]b[endmacro]\n[macro name=only_b][endmacro]\n")
            .build();
        let a = file(&db, "data/scenario/a.ks");
        let b = file(&db, "data/scenario/b.ks");

        let model_b = SemanticModel::new(&db, b);
        assert_eq!(
            model_b.resolve_macro("greet"),
            Some(MacroDef { file: b, name: "greet".to_string() }),
            "local definition shadows the project registry"
        );
        let model_a = SemanticModel::new(&db, a);
        assert_eq!(
            model_a.resolve_macro("only_b"),
            Some(MacroDef { file: b, name: "only_b".to_string() }),
            "project registry fills in cross-file macros"
        );
        assert_eq!(model_a.resolve_macro("nope"), None);
    }

    #[test]
    fn resolve_tag_builtin_beats_macro() {
        let db = ProjectBuilder::new()
            .file("data/scenario/a.ks", "[macro name=jump]shadow[endmacro]\n[macro name=greet][endmacro]\n")
            .build();
        let model = SemanticModel::new(&db, file(&db, "data/scenario/a.ks"));

        match model.resolve_tag("jump") {
            TagResolution::Builtin(spec) => assert_eq!(spec.name, "jump"),
            other => panic!("builtin must win, got {other:?}"),
        }
        match model.resolve_tag("greet") {
            TagResolution::Macro(def) => assert_eq!(def.name, "greet"),
            other => panic!("expected Macro, got {other:?}"),
        }
        assert_eq!(model.resolve_tag("zzz"), TagResolution::Unknown);
    }

    #[test]
    fn resolve_asset_checks_the_index() {
        let db = ProjectBuilder::new()
            .file("data/scenario/a.ks", "*a\n")
            .asset(AssetKind::BgImage, "room.jpg")
            .build();
        let model = SemanticModel::new(&db, file(&db, "data/scenario/a.ks"));

        assert_eq!(
            model.resolve_asset(AssetKind::BgImage, "room.jpg"),
            Some(ProjectPath::new("data/bgimage/room.jpg").unwrap())
        );
        assert_eq!(model.resolve_asset(AssetKind::BgImage, "gone.jpg"), None);
        assert_eq!(
            model.resolve_asset(AssetKind::Image, "room.jpg"),
            None,
            "namespaces do not leak into each other"
        );
    }
}
