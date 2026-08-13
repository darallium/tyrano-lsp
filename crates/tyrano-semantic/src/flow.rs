//! Bounded interprocedural execution-flow discovery.
//!
//! UI callbacks are reachability edges, not eagerly enumerated execution
//! sequences. A worklist expands each `(file, root-or-macro)` unit once, so
//! event cycles converge in `O(units + edges)` and cannot create a factorial
//! number of interleavings.

use std::collections::{HashMap, HashSet, VecDeque};

use tyrano_project::{File, ProjectDb, ProjectPath};
use tyrano_syntax::ast::InterpretOptions;
use tyrano_syntax::text::TextRange;

use crate::check::all_tags;
use crate::{SemanticModel, TagResolution, file_js_analysis};

/// One independently reachable body.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum FlowUnitKind {
    FileRoot,
    Macro(String),
    JavaScript,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FlowUnit {
    pub file: File,
    pub kind: FlowUnitKind,
    pub effect_count: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum FlowEdgeKind {
    Jump,
    Call,
    MacroCall,
    Event,
    Plugin,
    LoadJavaScript,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FlowEdge {
    pub from: usize,
    /// `None` means the destination expression is dynamic or unresolved.
    pub to: Option<usize>,
    pub kind: FlowEdgeKind,
    /// The destination was conservatively expanded from a dynamic value.
    pub dynamic: bool,
    pub site: TextRange,
}

/// Reachable program graph from `data/scenario/first.ks` (or the first
/// scenario when that conventional entry is absent).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ProjectFlowAnalysis {
    pub entry: Option<usize>,
    pub units: Vec<FlowUnit>,
    pub edges: Vec<FlowEdge>,
    /// Number of worklist expansions; useful for proving bounded convergence.
    pub iterations: usize,
}

impl ProjectFlowAnalysis {
    pub fn event_edge_count(&self) -> usize {
        self.edges
            .iter()
            .filter(|edge| edge.kind == FlowEdgeKind::Event)
            .count()
    }

    pub fn dynamic_edge_count(&self) -> usize {
        self.edges.iter().filter(|edge| edge.dynamic).count()
    }

    pub fn unresolved_edge_count(&self) -> usize {
        self.edges.iter().filter(|edge| edge.to.is_none()).count()
    }

    pub fn reachable_file_count(&self) -> usize {
        self.units
            .iter()
            .map(|unit| unit.file)
            .collect::<HashSet<_>>()
            .len()
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct UnitKey {
    file: File,
    kind: FlowUnitKind,
}

pub fn analyze_project_flow(db: &dyn ProjectDb) -> ProjectFlowAnalysis {
    let project = db.project();
    let Some(entry_file) =
        conventional_entry(db).or_else(|| project.scenario_files(db).first().copied())
    else {
        return ProjectFlowAnalysis::default();
    };

    let entry_key = UnitKey {
        file: entry_file,
        kind: FlowUnitKind::FileRoot,
    };
    let mut analysis = ProjectFlowAnalysis::default();
    let mut ids = HashMap::new();
    let entry = ensure_unit(db, entry_key, &mut analysis, &mut ids);
    analysis.entry = Some(entry);
    let mut queue = VecDeque::from([entry]);

    while let Some(unit_id) = queue.pop_front() {
        analysis.iterations += 1;
        let unit = analysis.units[unit_id].clone();
        if unit.kind == FlowUnitKind::JavaScript {
            continue;
        }
        let outgoing = discover_edges(db, &unit);
        for discovered in outgoing {
            if discovered.dynamic && discovered.target.is_none() {
                for target in conservative_targets(db, discovered.kind) {
                    let existed = ids.contains_key(&target);
                    let target_id = ensure_unit(db, target, &mut analysis, &mut ids);
                    if !existed {
                        queue.push_back(target_id);
                    }
                    analysis.edges.push(FlowEdge {
                        from: unit_id,
                        to: Some(target_id),
                        kind: discovered.kind,
                        dynamic: true,
                        site: discovered.site,
                    });
                }
                continue;
            }
            let target_id = discovered.target.map(|target| {
                let existed = ids.contains_key(&target);
                let id = ensure_unit(db, target, &mut analysis, &mut ids);
                if !existed {
                    queue.push_back(id);
                }
                id
            });
            analysis.edges.push(FlowEdge {
                from: unit_id,
                to: target_id,
                kind: discovered.kind,
                dynamic: discovered.dynamic,
                site: discovered.site,
            });
        }
    }
    analysis
}

fn conventional_entry(db: &dyn ProjectDb) -> Option<File> {
    let project = db.project();
    for root in &project.settings(db).scenario_roots {
        let path = root.join("first.ks").ok()?;
        if let Some(file) = db.files().get(&path) {
            return Some(file);
        }
    }
    None
}

fn ensure_unit(
    db: &dyn ProjectDb,
    key: UnitKey,
    analysis: &mut ProjectFlowAnalysis,
    ids: &mut HashMap<UnitKey, usize>,
) -> usize {
    if let Some(&id) = ids.get(&key) {
        return id;
    }
    let effect_count = effects_in_unit(db, &key);
    let id = analysis.units.len();
    analysis.units.push(FlowUnit {
        file: key.file,
        kind: key.kind.clone(),
        effect_count,
    });
    ids.insert(key, id);
    id
}

fn effects_in_unit(db: &dyn ProjectDb, key: &UnitKey) -> usize {
    let analysis = file_js_analysis(db, key.file);
    match &key.kind {
        FlowUnitKind::JavaScript => analysis.effect_count(),
        FlowUnitKind::FileRoot => analysis
            .regions
            .iter()
            .filter(|region| macro_at(db, key.file, region.range).is_none())
            .map(|region| region.facts.effects.len())
            .sum(),
        FlowUnitKind::Macro(name) => analysis
            .regions
            .iter()
            .filter(|region| macro_at(db, key.file, region.range).as_deref() == Some(name))
            .map(|region| region.facts.effects.len())
            .sum(),
    }
}

struct DiscoveredEdge {
    target: Option<UnitKey>,
    kind: FlowEdgeKind,
    dynamic: bool,
    site: TextRange,
}

fn discover_edges(db: &dyn ProjectDb, unit: &FlowUnit) -> Vec<DiscoveredEdge> {
    let source_file = unit.file.source(db);
    let module = tyrano_db::parsed_module(db, source_file);
    let options = source_file.interpret_options(db);
    let model = SemanticModel::new(db, unit.file);
    let mut edges = Vec::new();

    for tag in all_tags(&module.scenario()) {
        let site = tag.syntax().text_range();
        if !belongs_to_unit(db, unit, site) {
            continue;
        }
        let name = tag.name();
        let edge = match name.as_str() {
            "jump" => Some(scenario_edge(
                &tag,
                &options,
                &model,
                unit.file,
                FlowEdgeKind::Jump,
                site,
            )),
            "call" => Some(scenario_edge(
                &tag,
                &options,
                &model,
                unit.file,
                FlowEdgeKind::Call,
                site,
            )),
            "link" | "button" | "glink" | "clickable" => Some(scenario_edge(
                &tag,
                &options,
                &model,
                unit.file,
                FlowEdgeKind::Event,
                site,
            )),
            "plugin" => Some(auxiliary_edge(db, &tag, &options, true, site)),
            "loadjs" => Some(auxiliary_edge(db, &tag, &options, false, site)),
            _ => match model.resolve_tag(&name) {
                TagResolution::Macro(definition) => Some(DiscoveredEdge {
                    target: Some(UnitKey {
                        file: definition.file,
                        kind: FlowUnitKind::Macro(definition.name),
                    }),
                    kind: FlowEdgeKind::MacroCall,
                    dynamic: false,
                    site,
                }),
                _ => None,
            },
        };
        if let Some(edge) = edge {
            edges.push(edge);
        }
    }
    edges
}

fn scenario_edge(
    tag: &tyrano_syntax::ast::AnyTag,
    options: &InterpretOptions,
    model: &SemanticModel<'_>,
    current_file: File,
    kind: FlowEdgeKind,
    site: TextRange,
) -> DiscoveredEdge {
    let storage = tag
        .param("storage")
        .and_then(|param| param.cooked_value(options));
    let (target, dynamic) = match storage {
        None => (Some(current_file), false),
        Some(storage) if is_dynamic(&storage) => (None, true),
        Some(storage) => (model.resolve_storage(&storage), false),
    };
    DiscoveredEdge {
        target: target.map(|file| UnitKey {
            file,
            kind: FlowUnitKind::FileRoot,
        }),
        kind,
        dynamic,
        site,
    }
}

fn auxiliary_edge(
    db: &dyn ProjectDb,
    tag: &tyrano_syntax::ast::AnyTag,
    options: &InterpretOptions,
    plugin: bool,
    site: TextRange,
) -> DiscoveredEdge {
    let (path, edge_kind, unit_kind) = if plugin {
        let path = static_param(tag, "name", options).and_then(|name| {
            let storage =
                static_param(tag, "storage", options).unwrap_or_else(|| "init.ks".to_owned());
            ProjectPath::new(format!("data/others/plugin/{name}/{storage}")).ok()
        });
        (path, FlowEdgeKind::Plugin, FlowUnitKind::FileRoot)
    } else {
        let path = static_param(tag, "storage", options).and_then(|storage| {
            let storage = storage.strip_prefix("./").unwrap_or(&storage);
            ProjectPath::new(if storage.starts_with("data/") {
                storage.to_owned()
            } else {
                format!("data/others/{storage}")
            })
            .ok()
        });
        (path, FlowEdgeKind::LoadJavaScript, FlowUnitKind::JavaScript)
    };
    let dynamic = path.is_none()
        && if plugin {
            tag.param("name")
                .and_then(|param| param.cooked_value(options))
                .is_some_and(|value| is_dynamic(&value))
        } else {
            tag.param("storage")
                .and_then(|param| param.cooked_value(options))
                .is_some_and(|value| is_dynamic(&value))
        };
    DiscoveredEdge {
        target: path
            .and_then(|path| db.files().get(&path))
            .map(|file| UnitKey {
                file,
                kind: unit_kind,
            }),
        kind: edge_kind,
        dynamic,
        site,
    }
}

fn conservative_targets(db: &dyn ProjectDb, kind: FlowEdgeKind) -> Vec<UnitKey> {
    let project = db.project();
    let (files, unit_kind): (&[File], FlowUnitKind) = match kind {
        FlowEdgeKind::Jump | FlowEdgeKind::Call | FlowEdgeKind::Event => {
            (project.scenario_files(db), FlowUnitKind::FileRoot)
        }
        FlowEdgeKind::Plugin => (project.library_files(db), FlowUnitKind::FileRoot),
        FlowEdgeKind::LoadJavaScript => (project.javascript_files(db), FlowUnitKind::JavaScript),
        FlowEdgeKind::MacroCall => return Vec::new(),
    };
    files
        .iter()
        .copied()
        .map(|file| UnitKey {
            file,
            kind: unit_kind.clone(),
        })
        .collect()
}

fn static_param(
    tag: &tyrano_syntax::ast::AnyTag,
    name: &str,
    options: &InterpretOptions,
) -> Option<String> {
    let value = tag.param(name)?.cooked_value(options)?;
    (!is_dynamic(&value)).then_some(value)
}

fn is_dynamic(value: &str) -> bool {
    value.trim_start().starts_with(['&', '%'])
}

fn belongs_to_unit(db: &dyn ProjectDb, unit: &FlowUnit, range: TextRange) -> bool {
    match &unit.kind {
        FlowUnitKind::JavaScript => false,
        FlowUnitKind::FileRoot => macro_at(db, unit.file, range).is_none(),
        FlowUnitKind::Macro(name) => macro_at(db, unit.file, range).as_deref() == Some(name),
    }
}

pub(crate) fn macro_at(db: &dyn ProjectDb, file: File, range: TextRange) -> Option<String> {
    let index = tyrano_parser_core::semantic_index(db, file.source(db));
    index.scopes().iter().find_map(|scope| {
        let tyrano_parser_core::ScopeKind::MacroBody { def } = scope.kind else {
            return None;
        };
        if scope.range.start() <= range.start() && scope.range.end() >= range.end() {
            let definition = index.definition(def);
            Some(index.symbol(definition.symbol).name.clone())
        } else {
            None
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use tyrano_project::testing::ProjectBuilder;

    #[test]
    fn crosses_call_macro_and_event_boundaries_without_enumerating_orders() {
        let db = ProjectBuilder::new()
            .file(
                "data/scenario/first.ks",
                "[call storage=setup.ks]\n[greet]\n[button storage=menu.ks target=*open]\n[s]\n",
            )
            .file("data/scenario/setup.ks", "[return]\n")
            .file(
                "data/scenario/macros.ks",
                "[macro name=greet][eval exp=sf.greet=true][endmacro]\n",
            )
            .file(
                "data/scenario/menu.ks",
                "*open\n[button storage=first.ks target=*again]\n[s]\n",
            )
            .build();
        let flow = analyze_project_flow(&db);
        assert_eq!(flow.reachable_file_count(), 4);
        assert_eq!(flow.units.len(), 4, "three roots plus one macro body");
        assert_eq!(flow.iterations, flow.units.len());
        assert_eq!(flow.event_edge_count(), 2);
        assert!(
            flow.edges
                .iter()
                .any(|edge| edge.kind == FlowEdgeKind::Call)
        );
        assert!(
            flow.edges
                .iter()
                .any(|edge| edge.kind == FlowEdgeKind::MacroCall)
        );
    }

    #[test]
    fn resolves_plugin_and_loaded_javascript() {
        let db = ProjectBuilder::new()
            .file("data/scenario/first.ks", "[plugin name=demo]\n")
            .library(
                "data/others/plugin/demo/init.ks",
                "[loadjs storage=plugin/demo/main.js]\n",
            )
            .javascript("data/others/plugin/demo/main.js", "sf.demo = true;")
            .build();
        let flow = analyze_project_flow(&db);
        assert_eq!(flow.reachable_file_count(), 3);
        assert_eq!(flow.iterations, 3);
        assert!(
            flow.edges
                .iter()
                .any(|edge| edge.kind == FlowEdgeKind::Plugin)
        );
        assert!(
            flow.edges
                .iter()
                .any(|edge| edge.kind == FlowEdgeKind::LoadJavaScript)
        );
    }
}
