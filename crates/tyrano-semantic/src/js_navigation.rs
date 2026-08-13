//! Project-wide navigation index for owned JavaScript effects.
//!
//! This is intentionally downstream of `tyrano-js-analysis`: no Oxc AST or
//! arena reference is retained. Salsa caches only project `File` handles,
//! owned paths, and absolute byte ranges.

use std::cmp::Reverse;
use std::sync::Arc;

use tyrano_js_analysis::{EffectKind, EffectRole, PathSegment, VariablePath, VariableRoot};
use tyrano_project::{File, Project, ProjectDb};
use tyrano_syntax::text::{TextRange, TextSize};

use crate::analyze_project_javascript;
use crate::flow::macro_at;

/// A source occurrence returned to IDE adapters.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct JsSourceLocation {
    pub file: File,
    pub range: TextRange,
    pub kind: EffectKind,
    pub role: EffectRole,
}

/// Stable identity of a JavaScript binding or property path.
///
/// Engine/global paths can cross files and embedded-region boundaries.
/// Lexical paths additionally carry their owning file and region because Oxc
/// definition ranges are relative to an independently parsed region.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct JsSymbol {
    path: VariablePath,
    owner: SymbolOwner,
}

impl JsSymbol {
    pub fn path(&self) -> &VariablePath {
        &self.path
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct LexicalOwner {
    file: File,
    region_start: TextSize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum SymbolOwner {
    /// Engine/global state shared across source boundaries.
    Shared,
    /// One Oxc lexical environment, anchored by its independently parsed
    /// source region.
    Lexical(LexicalOwner),
    /// `mp` is recreated for each macro invocation; the macro definition is
    /// the finite context key used by navigation.
    MacroParameter { file: File, macro_name: String },
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct IndexedOccurrence {
    symbol: JsSymbol,
    location: JsSourceLocation,
    lexical_spelling: bool,
}

/// Salsa-safe, project-wide index of JavaScript occurrences.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ProjectJsNavigationIndex {
    occurrences: Vec<IndexedOccurrence>,
}

impl ProjectJsNavigationIndex {
    pub fn occurrence_count(&self) -> usize {
        self.occurrences.len()
    }
}

/// Builds a navigation projection over every scenario, library, and `.js`
/// source. Per-file parsing/lowering remains memoized by `file_js_analysis`.
#[salsa::tracked]
pub fn project_js_navigation_index(
    db: &dyn ProjectDb,
    project: Project,
) -> Arc<ProjectJsNavigationIndex> {
    // Explicitly read membership so adding or removing any source invalidates
    // the project projection even if all surviving per-file facts backdate.
    let _ = project.scenario_files(db);
    let _ = project.library_files(db);
    let _ = project.javascript_files(db);

    let mut occurrences = Vec::new();
    for (file, analysis) in analyze_project_javascript(db) {
        for region in &analysis.regions {
            for effect in &region.facts.effects {
                let lexical_owner = LexicalOwner {
                    file,
                    region_start: region.range.start(),
                };
                let owner = match &effect.path.root {
                    VariableRoot::Lexical { .. } => SymbolOwner::Lexical(lexical_owner),
                    VariableRoot::Engine(tyrano_js_analysis::EngineNamespace::Mp) => {
                        macro_at(db, file, region.range).map_or(SymbolOwner::Shared, |macro_name| {
                            SymbolOwner::MacroParameter { file, macro_name }
                        })
                    }
                    VariableRoot::Engine(_) | VariableRoot::Global(_) => SymbolOwner::Shared,
                };
                occurrences.push(IndexedOccurrence {
                    symbol: JsSymbol {
                        path: effect.path.clone(),
                        owner,
                    },
                    location: JsSourceLocation {
                        file,
                        range: absolute_range(region.range.start(), effect.selection),
                        kind: effect.kind,
                        role: effect.role,
                    },
                    lexical_spelling: false,
                });
            }
            for binding in &region.facts.bindings {
                occurrences.push(IndexedOccurrence {
                    symbol: JsSymbol {
                        path: VariablePath {
                            root: VariableRoot::Lexical {
                                name: binding.name.clone(),
                                definition: binding.definition,
                            },
                            segments: Vec::new(),
                        },
                        owner: SymbolOwner::Lexical(LexicalOwner {
                            file,
                            region_start: region.range.start(),
                        }),
                    },
                    location: JsSourceLocation {
                        file,
                        range: absolute_range(region.range.start(), binding.selection),
                        kind: if binding.declaration {
                            EffectKind::Define
                        } else {
                            EffectKind::Read
                        },
                        role: EffectRole::Value,
                    },
                    lexical_spelling: true,
                });
            }
        }
    }
    occurrences.sort_by(|a, b| occurrence_key(db, a).cmp(&occurrence_key(db, b)));
    Arc::new(ProjectJsNavigationIndex { occurrences })
}

/// Resolves a cursor position to the narrowest JavaScript binding/property
/// occurrence. Prefix dereferences lose to terminal value effects at the same
/// source token.
pub fn js_symbol_at(db: &dyn ProjectDb, file: File, offset: TextSize) -> Option<JsSymbol> {
    let index = project_js_navigation_index(db, db.project());
    index
        .occurrences
        .iter()
        .filter(|occurrence| {
            occurrence.location.file == file
                && !occurrence.location.range.is_empty()
                && occurrence.location.range.contains_inclusive(offset)
        })
        .min_by_key(|occurrence| {
            (
                u32::from(occurrence.location.range.len()),
                !occurrence.lexical_spelling,
                occurrence.location.role == EffectRole::Dereference,
                Reverse(occurrence.symbol.path.segments.len()),
            )
        })
        .map(|occurrence| occurrence.symbol.clone())
}

/// Definition candidates for `symbol`.
///
/// A lexical binding has a true `Define` occurrence. JavaScript object
/// properties do not; their best navigation targets are non-delete writes.
/// This distinction deliberately does not promote `Write` to `Define` in the
/// data-flow semantics.
pub fn js_definition_locations(db: &dyn ProjectDb, symbol: &JsSymbol) -> Vec<JsSourceLocation> {
    let lexical_binding =
        matches!(symbol.path.root, VariableRoot::Lexical { .. }) && symbol.path.segments.is_empty();
    collect_locations(db, symbol, |location| {
        if lexical_binding {
            location.kind == EffectKind::Define
        } else {
            location.kind == EffectKind::Write && location.role != EffectRole::Delete
        }
    })
}

/// Every read/write/execute occurrence of `symbol`, optionally including true
/// lexical declarations. Writes remain ordinary references for engine/global
/// properties because they are not declarations in JavaScript semantics.
pub fn js_reference_locations(
    db: &dyn ProjectDb,
    symbol: &JsSymbol,
    include_declaration: bool,
) -> Vec<JsSourceLocation> {
    collect_locations(db, symbol, |location| {
        include_declaration || location.kind != EffectKind::Define
    })
}

fn collect_locations(
    db: &dyn ProjectDb,
    symbol: &JsSymbol,
    include: impl Fn(&JsSourceLocation) -> bool,
) -> Vec<JsSourceLocation> {
    let index = project_js_navigation_index(db, db.project());
    let mut locations = index
        .occurrences
        .iter()
        .filter(|occurrence| symbols_may_alias(&occurrence.symbol, symbol))
        .map(|occurrence| &occurrence.location)
        .filter(|location| include(location))
        .cloned()
        .collect::<Vec<_>>();
    locations.dedup_by(|a, b| a.file == b.file && a.range == b.range);
    locations
}

fn symbols_may_alias(left: &JsSymbol, right: &JsSymbol) -> bool {
    if left.owner != right.owner {
        return false;
    }
    if !roots_match(&left.path.root, &right.path.root)
        || left.path.segments.len() != right.path.segments.len()
    {
        return false;
    }
    left.path
        .segments
        .iter()
        .zip(&right.path.segments)
        .all(|(left, right)| match (left, right) {
            (PathSegment::Static(left), PathSegment::Static(right)) => left == right,
            // A computed key may select any property. Returning a superset is
            // safer than silently hiding a possible definition/reference.
            (PathSegment::Dynamic(_), _) | (_, PathSegment::Dynamic(_)) => true,
        })
}

fn roots_match(left: &VariableRoot, right: &VariableRoot) -> bool {
    match (left, right) {
        (VariableRoot::Engine(left), VariableRoot::Engine(right)) => left == right,
        (VariableRoot::Global(left), VariableRoot::Global(right)) => left == right,
        (
            VariableRoot::Lexical {
                name: left_name,
                definition: left_definition,
            },
            VariableRoot::Lexical {
                name: right_name,
                definition: right_definition,
            },
        ) => left_name == right_name && left_definition == right_definition,
        _ => false,
    }
}

fn absolute_range(region_start: TextSize, range: tyrano_js_analysis::TextRange) -> TextRange {
    TextRange::new(
        region_start + TextSize::new(range.start),
        region_start + TextSize::new(range.end),
    )
}

fn occurrence_key<'a>(
    db: &'a dyn ProjectDb,
    occurrence: &'a IndexedOccurrence,
) -> (&'a tyrano_project::ProjectPath, TextSize, TextSize) {
    (
        occurrence.location.file.path(db),
        occurrence.location.range.start(),
        occurrence.location.range.end(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use tyrano_js_analysis::EngineNamespace;
    use tyrano_project::ProjectPath;
    use tyrano_project::testing::ProjectBuilder;

    fn file(db: &tyrano_project::ProjectDatabase, path: &str) -> File {
        db.file(&ProjectPath::new(path).unwrap()).unwrap()
    }

    fn offset(db: &tyrano_project::ProjectDatabase, file: File, needle: &str) -> TextSize {
        let text = file.source(db).text(db);
        TextSize::new(text.find(needle).unwrap() as u32)
    }

    #[test]
    fn lexical_reference_resolves_only_inside_its_js_region() {
        let db = ProjectBuilder::new()
            .file(
                "data/scenario/a.ks",
                "[iscript]\nlet local = 1; f.answer = local;\n[endscript]\n[eval exp='local + f.answer']\n",
            )
            .build();
        let file = file(&db, "data/scenario/a.ks");
        let source = file.source(&db).text(&db);
        let use_offset = TextSize::new(source.find("local;\n").unwrap() as u32);
        let symbol = js_symbol_at(&db, file, use_offset).expect("lexical use");
        let definitions = js_definition_locations(&db, &symbol);
        assert_eq!(definitions.len(), 1, "{definitions:#?}");
        assert_eq!(
            &source[usize::from(definitions[0].range.start())
                ..usize::from(definitions[0].range.end())],
            "local"
        );
        assert_eq!(js_reference_locations(&db, &symbol, false).len(), 1);
    }

    #[test]
    fn engine_property_references_cross_region_and_file_boundaries() {
        let db = ProjectBuilder::new()
            .file(
                "data/scenario/a.ks",
                "[iscript]\nf.player.score = 1;\n[endscript]\n",
            )
            .file(
                "data/scenario/b.ks",
                "[if cond='TYRANO.kag.stat.f.player.score > 0']ok[endif]\n",
            )
            .javascript(
                "data/others/plugin/demo.js",
                "const state = f.player; console.log(state.score);\n",
            )
            .build();
        let b = file(&db, "data/scenario/b.ks");
        let symbol = js_symbol_at(&db, b, offset(&db, b, "score")).expect("engine property");
        assert_eq!(symbol.path.root, VariableRoot::Engine(EngineNamespace::F));
        let definitions = js_definition_locations(&db, &symbol);
        assert_eq!(definitions.len(), 1, "{definitions:#?}");
        assert_eq!(definitions[0].file.path(&db).as_str(), "data/scenario/a.ks");

        let references = js_reference_locations(&db, &symbol, false);
        let paths = references
            .iter()
            .map(|location| location.file.path(&db).as_str())
            .collect::<Vec<_>>();
        assert_eq!(
            paths,
            [
                "data/others/plugin/demo.js",
                "data/scenario/a.ks",
                "data/scenario/b.ks"
            ]
        );
    }

    #[test]
    fn selecting_parent_and_terminal_properties_yields_different_symbols() {
        let db = ProjectBuilder::new()
            .file(
                "data/scenario/a.ks",
                "[eval exp='f.player.hp = f.player.max_hp']\n",
            )
            .build();
        let file = file(&db, "data/scenario/a.ks");
        let player = js_symbol_at(&db, file, offset(&db, file, "player")).unwrap();
        let hp = js_symbol_at(&db, file, offset(&db, file, "hp =")).unwrap();
        assert_eq!(player.path.segments.len(), 1);
        assert_eq!(hp.path.segments.len(), 2);
    }

    #[test]
    fn alias_spelling_keeps_lexical_navigation_identity() {
        let db = ProjectBuilder::new()
            .javascript(
                "data/others/state.js",
                "const state = sf.game; state.score = 1; console.log(state.score);\n",
            )
            .build();
        let file = file(&db, "data/others/state.js");
        let source = file.source(&db).text(&db);
        let use_offset = TextSize::new(source.find("state.score =").unwrap() as u32);
        let symbol = js_symbol_at(&db, file, use_offset).unwrap();
        assert!(matches!(symbol.path.root, VariableRoot::Lexical { .. }));
        assert!(symbol.path.segments.is_empty());
        let definitions = js_definition_locations(&db, &symbol);
        assert_eq!(definitions.len(), 1, "{definitions:#?}");
        assert_eq!(definitions[0].range.start(), TextSize::new(6));

        let score_offset = use_offset + TextSize::new("state.".len() as u32);
        let score = js_symbol_at(&db, file, score_offset).unwrap();
        assert_eq!(score.path.root, VariableRoot::Engine(EngineNamespace::Sf));
        assert_eq!(
            score.path.segments,
            [
                PathSegment::Static("game".to_owned()),
                PathSegment::Static("score".to_owned())
            ]
        );
    }

    #[test]
    fn macro_parameter_paths_are_scoped_per_macro_definition() {
        let db = ProjectBuilder::new()
            .file(
                "data/scenario/macros.ks",
                "[macro name=one][eval exp='mp.value'][endmacro]\n[macro name=two][eval exp='mp.value'][endmacro]\n",
            )
            .build();
        let file = file(&db, "data/scenario/macros.ks");
        let symbol = js_symbol_at(&db, file, offset(&db, file, "value")).unwrap();
        let references = js_reference_locations(&db, &symbol, false);
        assert_eq!(references.len(), 1, "{references:#?}");
    }
}
