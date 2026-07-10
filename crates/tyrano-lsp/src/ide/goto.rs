//! Go to definition, across files.

use tyrano_parser_core::{RefKind, SymbolKind, semantic_index};
use tyrano_project::registry::{ValueKind, builtin_registry};
use tyrano_project::{File, ProjectDb};
use tyrano_semantic::{ProjectResolution, SemanticModel, TagResolution};
use tyrano_syntax::text::{TextRange, TextSize};

use super::NavTarget;
use super::cursor::{CursorTarget, classify};

/// Definition sites for the construct at `offset` in `file`.
pub fn goto_definition(db: &dyn ProjectDb, file: File, offset: TextSize) -> Vec<NavTarget> {
    let model = SemanticModel::new(db, file);
    match classify(db, file, offset) {
        // The cursor already sits on a definition; nowhere else to go.
        CursorTarget::Def { .. } => Vec::new(),
        CursorTarget::Use { index: i, .. } => {
            let index = semantic_index(db, file.source(db));
            let use_ = &index.uses()[i];
            match &model.resolved_references()[i] {
                ProjectResolution::Local(def) => {
                    vec![def_target(db, file, *def)]
                }
                ProjectResolution::ExternalLabel(target) => {
                    vec![def_target(db, target.file, target.def)]
                }
                ProjectResolution::ExternalFile(target)
                | ProjectResolution::LabelNotFound { file: target, .. } => {
                    vec![file_target(db, *target)]
                }
                ProjectResolution::FileNotFound { .. } => Vec::new(),
                ProjectResolution::Unknown => match use_.kind {
                    // Characters are routinely defined in other files; the
                    // file-local index cannot see those definitions.
                    RefKind::CharacterRef => character_defs(db, &use_.name),
                    _ => Vec::new(),
                },
            }
        }
        CursorTarget::Tag { name, .. } => match model.resolve_tag(&name) {
            TagResolution::Macro(def) => macro_def_target(db, def.file, &def.name)
                .map(|t| vec![t])
                .unwrap_or_default(),
            _ => Vec::new(),
        },
        CursorTarget::ParamValue { tag, param, value, .. } => {
            value_targets(db, &model, &tag, &param, &value)
        }
        CursorTarget::ParamName { .. } | CursorTarget::None => Vec::new(),
    }
}

/// A target pointing at a definition's name range.
fn def_target(db: &dyn ProjectDb, file: File, def: tyrano_parser_core::DefinitionId) -> NavTarget {
    let index = semantic_index(db, file.source(db));
    NavTarget { path: file.path(db).clone(), range: index.definition(def).name_range }
}

/// A target pointing at the top of a file.
fn file_target(db: &dyn ProjectDb, file: File) -> NavTarget {
    NavTarget { path: file.path(db).clone(), range: TextRange::empty(0.into()) }
}

/// The winning `[macro name=…]` definition site of `name` in `file`.
fn macro_def_target(db: &dyn ProjectDb, file: File, name: &str) -> Option<NavTarget> {
    let index = semantic_index(db, file.source(db));
    let symbol = index.macro_(name)?;
    let def = *index.symbol(symbol).defs.first()?;
    Some(def_target(db, file, def))
}

/// Definition sites of character `name` across every scenario file.
fn character_defs(db: &dyn ProjectDb, name: &str) -> Vec<NavTarget> {
    let mut out = Vec::new();
    for &file in db.project().scenario_files(db) {
        let index = semantic_index(db, file.source(db));
        if let Some(symbol) = index.character(name) {
            debug_assert_eq!(index.symbol(symbol).kind, SymbolKind::Character);
            for &def in &index.symbol(symbol).defs {
                out.push(def_target(db, file, def));
            }
        }
    }
    out
}

/// Targets for a parameter value: `storage=` files and asset references.
fn value_targets(
    db: &dyn ProjectDb,
    model: &SemanticModel<'_>,
    tag: &str,
    param: &str,
    value: &str,
) -> Vec<NavTarget> {
    if value.starts_with('&') || value.starts_with('%') {
        return Vec::new();
    }
    let Some(spec) = builtin_registry().get(tag) else { return Vec::new() };
    let Some(param) = spec.param(param) else { return Vec::new() };
    match param.kind {
        ValueKind::Scenario => model
            .resolve_storage(value)
            .map(|f| vec![file_target(db, f)])
            .unwrap_or_default(),
        ValueKind::Asset(kind) => model
            .resolve_asset(kind, value)
            .map(|path| vec![NavTarget { path, range: TextRange::empty(0.into()) }])
            .unwrap_or_default(),
        _ => Vec::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ide::testutil::{file, offset, project};
    use tyrano_project::testing::ProjectBuilder;
    use tyrano_project::{AssetKind, ProjectPath};

    fn nav(db: &tyrano_project::ProjectDatabase, f: File, needle: &str, add: u32) -> Vec<NavTarget> {
        goto_definition(db, f, offset(db, f, needle, add))
    }

    #[test]
    fn local_jump_target_goes_to_label() {
        let db = project(&[("data/scenario/a.ks", "*start\n[jump target=*start]\n")]);
        let f = file(&db, "data/scenario/a.ks");
        let targets = nav(&db, f, "target=*start", 10);
        assert_eq!(targets.len(), 1);
        assert_eq!(targets[0].path.as_str(), "data/scenario/a.ks");
        // The label's name range is "start" right after the leading `*`.
        assert_eq!(u32::from(targets[0].range.start()), 1);
    }

    #[test]
    fn cross_file_jump_target_goes_to_other_file() {
        let db = project(&[
            ("data/scenario/a.ks", "[jump storage=b.ks target=*top]\n"),
            ("data/scenario/b.ks", "; intro\n*top\n"),
        ]);
        let f = file(&db, "data/scenario/a.ks");
        let targets = nav(&db, f, "*top", 2);
        assert_eq!(targets.len(), 1);
        assert_eq!(targets[0].path.as_str(), "data/scenario/b.ks");
        let b = file(&db, "data/scenario/b.ks");
        let text = b.source(&db).text(&db);
        assert_eq!(&text[targets[0].range.start().into()..targets[0].range.end().into()], "top");
    }

    #[test]
    fn storage_value_goes_to_file_top() {
        let db = project(&[
            ("data/scenario/a.ks", "[jump storage=b.ks]\n"),
            ("data/scenario/b.ks", "*top\n"),
        ]);
        let f = file(&db, "data/scenario/a.ks");
        // On the use itself (jump without target records an external use of
        // the whole tag) — classify may see the storage value via CST; both
        // paths must land in b.ks.
        let targets = nav(&db, f, "b.ks", 1);
        assert_eq!(targets.len(), 1, "{targets:?}");
        assert_eq!(targets[0].path.as_str(), "data/scenario/b.ks");
    }

    #[test]
    fn cross_file_macro_call_goes_to_definition() {
        let db = project(&[
            ("data/scenario/a.ks", "[only_b]\n"),
            ("data/scenario/b.ks", "[macro name=only_b][endmacro]\n"),
        ]);
        let f = file(&db, "data/scenario/a.ks");
        let targets = nav(&db, f, "[only_b]", 3);
        assert_eq!(targets.len(), 1);
        assert_eq!(targets[0].path.as_str(), "data/scenario/b.ks");
        let b = file(&db, "data/scenario/b.ks");
        let text = b.source(&db).text(&db);
        assert_eq!(
            &text[targets[0].range.start().into()..targets[0].range.end().into()],
            "only_b"
        );
    }

    #[test]
    fn character_ref_finds_cross_file_definition() {
        let db = project(&[
            ("data/scenario/a.ks", "[chara_show name=akane]\n"),
            ("data/scenario/chars.ks", "[chara_new name=akane storage=akane.png]\n"),
        ]);
        let f = file(&db, "data/scenario/a.ks");
        let targets = nav(&db, f, "akane]", 2);
        assert_eq!(targets.len(), 1, "{targets:?}");
        assert_eq!(targets[0].path.as_str(), "data/scenario/chars.ks");
    }

    #[test]
    fn asset_value_goes_to_asset_path() {
        let db = ProjectBuilder::new()
            .file("data/scenario/a.ks", "[bg storage=room.jpg]\n")
            .asset(AssetKind::BgImage, "room.jpg")
            .build();
        let f = db.file(&ProjectPath::new("data/scenario/a.ks").unwrap()).unwrap();
        let targets = goto_definition(&db, f, offset(&db, f, "room.jpg", 2));
        assert_eq!(targets.len(), 1, "{targets:?}");
        assert_eq!(targets[0].path.as_str(), "data/bgimage/room.jpg");
    }

    #[test]
    fn missing_target_navigates_nowhere_or_to_file() {
        let db = project(&[("data/scenario/a.ks", "[jump storage=gone.ks target=*x]\n")]);
        let f = file(&db, "data/scenario/a.ks");
        assert!(nav(&db, f, "*x]", 1).is_empty(), "missing file: no target");
    }
}
