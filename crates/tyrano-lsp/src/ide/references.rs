//! Find all references, project-wide.

use tyrano_parser_core::{DefinitionId, RefKind, Resolution, SymbolKind, semantic_index};
use tyrano_project::{File, ProjectDb};
use tyrano_semantic::{ProjectResolution, SemanticModel, TagResolution};
use tyrano_syntax::ast::AnyTag;
use tyrano_syntax::text::TextSize;

use super::NavTarget;
use super::cursor::{CursorTarget, classify};

/// The symbol whose references we are collecting.
#[derive(Debug, Clone, PartialEq, Eq)]
enum RefTarget {
    /// A label definition `(file, def)`.
    Label(File, DefinitionId),
    /// A macro `(defining file, name)`.
    Macro(File, String),
    /// A character name (defined anywhere).
    Character(String),
}

/// Every reference to the symbol at `offset`, across the whole project.
/// Includes the definition site(s) when `include_declaration` is set.
pub fn references(
    db: &dyn ProjectDb,
    file: File,
    offset: TextSize,
    include_declaration: bool,
) -> Vec<NavTarget> {
    let Some(target) = ref_target(db, file, offset) else { return Vec::new() };

    let mut out = Vec::new();
    if include_declaration {
        collect_declarations(db, &target, &mut out);
    }
    for &scenario in db.project().scenario_files(db) {
        collect_in_file(db, scenario, &target, &mut out);
    }
    out
}

/// Resolves whatever is under the cursor to a [`RefTarget`].
fn ref_target(db: &dyn ProjectDb, file: File, offset: TextSize) -> Option<RefTarget> {
    let model = SemanticModel::new(db, file);
    let index = semantic_index(db, file.source(db));
    match classify(db, file, offset) {
        CursorTarget::Def { def, .. } => {
            let symbol = index.symbol(index.definition(def).symbol);
            Some(match symbol.kind {
                SymbolKind::Label => RefTarget::Label(file, symbol.defs[0]),
                SymbolKind::Macro => RefTarget::Macro(file, symbol.name.clone()),
                SymbolKind::Character => RefTarget::Character(symbol.name.clone()),
            })
        }
        CursorTarget::Use { index: i, .. } => {
            let use_ = &index.uses()[i];
            match use_.kind {
                RefKind::CharacterRef => Some(RefTarget::Character(use_.name.clone())),
                RefKind::MacroCall => {
                    let def = model.resolve_macro(&use_.name)?;
                    Some(RefTarget::Macro(def.file, def.name))
                }
                RefKind::JumpTarget => match &model.resolved_references()[i] {
                    ProjectResolution::Local(def) => Some(RefTarget::Label(file, *def)),
                    ProjectResolution::ExternalLabel(t) => Some(RefTarget::Label(t.file, t.def)),
                    _ => None,
                },
            }
        }
        CursorTarget::Tag { name, .. } => match model.resolve_tag(&name) {
            TagResolution::Macro(def) => Some(RefTarget::Macro(def.file, def.name)),
            _ => None,
        },
        _ => None,
    }
}

fn collect_declarations(db: &dyn ProjectDb, target: &RefTarget, out: &mut Vec<NavTarget>) {
    match target {
        RefTarget::Label(file, def) => {
            let index = semantic_index(db, file.source(db));
            out.push(NavTarget {
                path: file.path(db).clone(),
                range: index.definition(*def).name_range,
            });
        }
        RefTarget::Macro(file, name) => {
            let index = semantic_index(db, file.source(db));
            if let Some(symbol) = index.macro_(name) {
                let def = index.symbol(symbol).defs[0];
                out.push(NavTarget {
                    path: file.path(db).clone(),
                    range: index.definition(def).name_range,
                });
            }
        }
        RefTarget::Character(name) => {
            for &file in db.project().scenario_files(db) {
                let index = semantic_index(db, file.source(db));
                if let Some(symbol) = index.character(name) {
                    for &def in &index.symbol(symbol).defs {
                        out.push(NavTarget {
                            path: file.path(db).clone(),
                            range: index.definition(def).name_range,
                        });
                    }
                }
            }
        }
    }
}

/// References to `target` inside `scenario`, in document order.
fn collect_in_file(
    db: &dyn ProjectDb,
    scenario: File,
    target: &RefTarget,
    out: &mut Vec<NavTarget>,
) {
    let index = semantic_index(db, scenario.source(db));
    let path = scenario.path(db);

    match target {
        RefTarget::Label(def_file, def) => {
            let model = SemanticModel::new(db, scenario);
            let resolved = model.resolved_references();
            for (use_, resolution) in index.uses().iter().zip(&resolved) {
                let hit = match resolution {
                    ProjectResolution::Local(d) => scenario == *def_file && d == def,
                    ProjectResolution::ExternalLabel(t) => {
                        t.file == *def_file && t.def == *def
                    }
                    _ => false,
                };
                if hit {
                    out.push(NavTarget { path: path.clone(), range: use_.range });
                }
            }
        }
        RefTarget::Macro(def_file, name) => {
            // Local calls are recorded as uses; cross-file calls are plain
            // unknown tags in the CST, resolved through the project macro
            // registry on demand.
            if scenario == *def_file {
                for use_ in index.uses() {
                    if use_.kind == RefKind::MacroCall
                        && use_.name == *name
                        && matches!(use_.resolution, Resolution::Def(_))
                    {
                        out.push(NavTarget { path: path.clone(), range: use_.range });
                    }
                }
            } else {
                let model = SemanticModel::new(db, scenario);
                let resolves_here = model
                    .resolve_macro(name)
                    .is_some_and(|d| d.file == *def_file && d.name == *name);
                if !resolves_here {
                    return;
                }
                let module = tyrano_db::parsed_module(db, scenario.source(db));
                for node in module.syntax().descendants() {
                    let Some(tag) = AnyTag::cast(node) else { continue };
                    if tag.name() != *name {
                        continue;
                    }
                    if let Some(token) = tag.tag_name().and_then(|n| n.token()) {
                        out.push(NavTarget { path: path.clone(), range: token.text_range() });
                    }
                }
            }
        }
        RefTarget::Character(name) => {
            for use_ in index.uses() {
                if use_.kind == RefKind::CharacterRef && use_.name == *name {
                    out.push(NavTarget { path: path.clone(), range: use_.range });
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ide::testutil::{file, offset, project};

    #[test]
    fn label_references_span_files() {
        let db = project(&[
            ("data/scenario/a.ks", "*top\n[jump target=*top]\n"),
            ("data/scenario/b.ks", "[jump storage=a.ks target=*top]\n"),
        ]);
        let a = file(&db, "data/scenario/a.ks");
        // From the definition site.
        let refs = references(&db, a, offset(&db, a, "*top", 2), true);
        let paths: Vec<&str> = refs.iter().map(|t| t.path.as_str()).collect();
        assert_eq!(
            paths,
            ["data/scenario/a.ks", "data/scenario/a.ks", "data/scenario/b.ks"],
            "decl + local use + cross-file use: {refs:?}"
        );

        // From a cross-file use site, without the declaration.
        let b = file(&db, "data/scenario/b.ks");
        let refs = references(&db, b, offset(&db, b, "*top", 2), false);
        assert_eq!(refs.len(), 2, "{refs:?}");
    }

    #[test]
    fn macro_references_span_files() {
        let db = project(&[
            ("data/scenario/a.ks", "[macro name=greet]hi[endmacro]\n[greet]\n"),
            ("data/scenario/b.ks", "[greet]\n[greet]\n"),
        ]);
        let a = file(&db, "data/scenario/a.ks");
        let refs = references(&db, a, offset(&db, a, "name=greet", 5), true);
        let paths: Vec<&str> = refs.iter().map(|t| t.path.as_str()).collect();
        assert_eq!(
            paths,
            [
                "data/scenario/a.ks", // declaration
                "data/scenario/a.ks", // local call
                "data/scenario/b.ks",
                "data/scenario/b.ks",
            ],
            "{refs:?}"
        );
    }

    #[test]
    fn shadowed_macro_calls_do_not_count() {
        // b.ks defines its own `greet`, so its calls are not references to
        // a.ks's macro.
        let db = project(&[
            ("data/scenario/a.ks", "[macro name=greet]a[endmacro]\n[greet]\n"),
            ("data/scenario/b.ks", "[macro name=greet]b[endmacro]\n[greet]\n"),
        ]);
        let a = file(&db, "data/scenario/a.ks");
        let refs = references(&db, a, offset(&db, a, "name=greet", 5), false);
        let paths: Vec<&str> = refs.iter().map(|t| t.path.as_str()).collect();
        assert_eq!(paths, ["data/scenario/a.ks"], "{refs:?}");
    }

    #[test]
    fn character_references_span_files() {
        let db = project(&[
            ("data/scenario/chars.ks", "[chara_new name=akane storage=a.png]\n"),
            ("data/scenario/a.ks", "[chara_show name=akane]\n[chara_hide name=akane]\n"),
        ]);
        let chars = file(&db, "data/scenario/chars.ks");
        let refs = references(&db, chars, offset(&db, chars, "akane", 2), true);
        let paths: Vec<&str> = refs.iter().map(|t| t.path.as_str()).collect();
        assert_eq!(
            paths,
            ["data/scenario/chars.ks", "data/scenario/a.ks", "data/scenario/a.ks"],
            "{refs:?}"
        );
    }
}
