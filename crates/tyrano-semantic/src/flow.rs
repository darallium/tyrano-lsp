//! Bounded interprocedural execution-flow discovery.
//!
//! UI callbacks are reachability edges, not eagerly enumerated execution
//! sequences. A worklist expands each `(file, root-or-macro)` unit once, so
//! event cycles converge in `O(units + edges)` and cannot create a factorial
//! number of interleavings.

use std::collections::{BTreeMap, HashMap, HashSet, VecDeque};

use tyrano_project::{File, ProjectDb, ProjectPath};
use tyrano_syntax::ast::InterpretOptions;
use tyrano_syntax::text::TextRange;

use crate::check::all_tags;
use crate::check::kind::{ValueClass, classify};
use crate::{
    SemanticModel, TagResolution, ValueCandidates, file_js_analysis,
    value_candidates_for_parameter_with_mp,
};

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
    /// Data-flow edge created by a macro-call `*` argument.
    MacroParameterForward,
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
    /// Macro argument values and their caller-`mp` dependencies. Empty for
    /// non-macro edges and for the separate `MacroParameterForward` edge.
    pub arguments: Vec<FlowArgument>,
    pub site: TextRange,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FlowArgument {
    pub name: String,
    pub candidates: Option<ValueCandidates>,
    pub depends_on: Vec<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct MacroValueBindings {
    pub by_macro: BTreeMap<String, BTreeMap<String, ValueCandidates>>,
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
    let macro_values = analyze_macro_value_bindings(db);

    while let Some(unit_id) = queue.pop_front() {
        analysis.iterations += 1;
        let unit = analysis.units[unit_id].clone();
        if unit.kind == FlowUnitKind::JavaScript {
            continue;
        }
        let outgoing = discover_edges(db, &unit, &macro_values);
        for discovered in outgoing {
            if discovered.conservative && discovered.target.is_none() {
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
                        arguments: discovered.arguments.clone(),
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
                arguments: discovered.arguments,
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
    conservative: bool,
    arguments: Vec<FlowArgument>,
    site: TextRange,
}

fn discover_edges(
    db: &dyn ProjectDb,
    unit: &FlowUnit,
    macro_values: &MacroValueBindings,
) -> Vec<DiscoveredEdge> {
    let source_file = unit.file.source(db);
    let module = tyrano_db::parsed_module(db, source_file);
    let options = source_file.interpret_options(db);
    let model = SemanticModel::new(db, unit.file);
    let mut edges = Vec::new();
    let mp = match &unit.kind {
        FlowUnitKind::Macro(name) => macro_values.by_macro.get(name).cloned().unwrap_or_default(),
        _ => BTreeMap::new(),
    };

    for tag in all_tags(&module.scenario()) {
        let site = tag.syntax().text_range();
        if !belongs_to_unit(db, unit, site) {
            continue;
        }
        let name = tag.name();
        match name.as_str() {
            "jump" => edges.extend(scenario_edges(
                db,
                &tag,
                &options,
                &model,
                FlowEdgeKind::Jump,
                site,
                &mp,
            )),
            "call" => edges.extend(scenario_edges(
                db,
                &tag,
                &options,
                &model,
                FlowEdgeKind::Call,
                site,
                &mp,
            )),
            "link" | "button" | "glink" | "clickable" => edges.extend(scenario_edges(
                db,
                &tag,
                &options,
                &model,
                FlowEdgeKind::Event,
                site,
                &mp,
            )),
            "plugin" => edges.push(auxiliary_edge(db, &tag, &options, true, site)),
            "loadjs" => edges.push(auxiliary_edge(db, &tag, &options, false, site)),
            _ => {
                if let TagResolution::Macro(definition) = model.resolve_tag(&name) {
                    let target = UnitKey {
                        file: definition.file,
                        kind: FlowUnitKind::Macro(definition.name),
                    };
                    edges.push(DiscoveredEdge {
                        target: Some(target.clone()),
                        kind: FlowEdgeKind::MacroCall,
                        dynamic: false,
                        conservative: false,
                        arguments: macro_arguments(db, unit.file, &tag, &options, &mp),
                        site,
                    });
                    if tag.params().iter().any(|param| param.is_macro_star()) {
                        edges.push(DiscoveredEdge {
                            target: Some(target),
                            kind: FlowEdgeKind::MacroParameterForward,
                            dynamic: false,
                            conservative: false,
                            arguments: Vec::new(),
                            site,
                        });
                    }
                }
            }
        }
    }
    edges
}

fn scenario_edges(
    db: &dyn ProjectDb,
    tag: &tyrano_syntax::ast::AnyTag,
    options: &InterpretOptions,
    model: &SemanticModel<'_>,
    kind: FlowEdgeKind,
    site: TextRange,
    mp: &BTreeMap<String, ValueCandidates>,
) -> Vec<DiscoveredEdge> {
    let storage = tag
        .param("storage")
        .and_then(|param| param.cooked_value(options));
    match storage {
        None => vec![scenario_discovered(
            Some(model.file()),
            kind,
            false,
            false,
            site,
        )],
        Some(storage) if !is_dynamic(&storage) => vec![scenario_discovered(
            model.resolve_storage(&storage),
            kind,
            false,
            false,
            site,
        )],
        Some(_) => {
            let candidates =
                value_candidates_for_parameter_with_mp(db, model.file(), site, "storage", mp)
                    .filter(|candidates| candidates.is_definitely_finite_strings());
            let Some(candidates) = candidates else {
                return vec![scenario_discovered(None, kind, true, true, site)];
            };
            let mut edges = candidates
                .string_values()
                .map(|storage| {
                    scenario_discovered(model.resolve_storage(storage), kind, true, false, site)
                })
                .collect::<Vec<_>>();
            if edges.is_empty() {
                edges.push(scenario_discovered(None, kind, true, false, site));
            }
            edges
        }
    }
}

fn scenario_discovered(
    target: Option<File>,
    kind: FlowEdgeKind,
    dynamic: bool,
    conservative: bool,
    site: TextRange,
) -> DiscoveredEdge {
    DiscoveredEdge {
        target: target.map(|file| UnitKey {
            file,
            kind: FlowUnitKind::FileRoot,
        }),
        kind,
        dynamic,
        conservative,
        arguments: Vec::new(),
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
        conservative: dynamic,
        arguments: Vec::new(),
        site,
    }
}

/// Fixed-point value propagation across macro-call edges. Explicit arguments
/// override `*` for each call, while values from distinct call sites join.
pub fn analyze_macro_value_bindings(db: &dyn ProjectDb) -> MacroValueBindings {
    let project = db.project();
    let mut files = project.scenario_files(db).clone();
    files.extend(project.library_files(db));
    files.sort_by(|left, right| left.path(db).cmp(right.path(db)));
    files.dedup();
    let macro_count = tyrano_project::project_macros(db).len();
    let mut result = MacroValueBindings::default();

    for _ in 0..macro_count.saturating_mul(32).max(1) {
        let before = result.clone();
        for &file in &files {
            let source_file = file.source(db);
            let module = tyrano_db::parsed_module(db, source_file);
            let options = source_file.interpret_options(db);
            let model = SemanticModel::new(db, file);
            for tag in all_tags(&module.scenario()) {
                let TagResolution::Macro(target) = model.resolve_tag(&tag.name()) else {
                    continue;
                };
                let site = tag.syntax().text_range();
                let caller_name = macro_at(db, file, site);
                let caller = caller_name
                    .as_ref()
                    .and_then(|name| result.by_macro.get(name))
                    .cloned()
                    .unwrap_or_default();
                let mut contribution = if tag.params().iter().any(|param| param.is_macro_star()) {
                    caller.clone()
                } else {
                    BTreeMap::new()
                };
                for argument in macro_arguments(db, file, &tag, &options, &caller) {
                    if let Some(candidates) = argument.candidates {
                        contribution.insert(argument.name, candidates);
                    }
                }
                let target_values = result.by_macro.entry(target.name).or_default();
                for (name, candidates) in contribution {
                    target_values
                        .entry(name)
                        .and_modify(|known| known.join(&candidates))
                        .or_insert(candidates);
                }
            }
        }
        if result == before {
            break;
        }
    }
    result
}

fn macro_arguments(
    db: &dyn ProjectDb,
    file: File,
    tag: &tyrano_syntax::ast::AnyTag,
    options: &InterpretOptions,
    mp: &BTreeMap<String, ValueCandidates>,
) -> Vec<FlowArgument> {
    tag.params()
        .into_iter()
        .filter(|param| !param.is_macro_star())
        .map(|param| {
            let depends_on = macro_parameter_dependencies(db, file, tag, &param, options);
            let candidates = argument_candidates(db, file, tag, &param, options, mp, &depends_on);
            FlowArgument {
                name: param.name(),
                candidates,
                depends_on,
            }
        })
        .collect()
}

fn argument_candidates(
    db: &dyn ProjectDb,
    file: File,
    tag: &tyrano_syntax::ast::AnyTag,
    param: &tyrano_syntax::ast::Param,
    options: &InterpretOptions,
    mp: &BTreeMap<String, ValueCandidates>,
    dependencies: &[String],
) -> Option<ValueCandidates> {
    match classify(param, options) {
        ValueClass::Static(value) => Some(ValueCandidates::string(value)),
        ValueClass::ParamRef => {
            let cooked = param.cooked_value(options).unwrap_or_default();
            let reference = cooked.trim_start().strip_prefix('%').unwrap_or(&cooked);
            let (name, fallback) = reference
                .split_once('|')
                .map_or((reference, None), |(name, fallback)| (name, Some(fallback)));
            mp.get(name)
                .cloned()
                .or_else(|| fallback.map(ValueCandidates::string))
        }
        ValueClass::Entity => {
            if dependencies.iter().any(|name| !mp.contains_key(name)) {
                return None;
            }
            value_candidates_for_parameter_with_mp(
                db,
                file,
                tag.syntax().text_range(),
                &param.name(),
                mp,
            )
        }
        ValueClass::Absent => None,
    }
}

fn macro_parameter_dependencies(
    db: &dyn ProjectDb,
    file: File,
    tag: &tyrano_syntax::ast::AnyTag,
    param: &tyrano_syntax::ast::Param,
    options: &InterpretOptions,
) -> Vec<String> {
    if matches!(classify(param, options), ValueClass::ParamRef) {
        let cooked = param.cooked_value(options).unwrap_or_default();
        let name = cooked
            .trim_start()
            .strip_prefix('%')
            .unwrap_or(&cooked)
            .split('|')
            .next()
            .unwrap_or("");
        return if name.is_empty() {
            Vec::new()
        } else {
            vec![name.to_owned()]
        };
    }
    if !matches!(classify(param, options), ValueClass::Entity) {
        return Vec::new();
    }
    let tag_range = tag.syntax().text_range();
    let analysis = file_js_analysis(db, file);
    let mut dependencies = analysis
        .regions
        .iter()
        .filter(|region| {
            tag_range.start() <= region.range.start()
                && tag_range.end() >= region.range.end()
                && matches!(
                    &region.kind,
                    crate::JsRegionKind::Entity { parameter, .. }
                        | crate::JsRegionKind::Parameter { parameter, .. }
                        if parameter == &param.name()
                )
        })
        .flat_map(|region| &region.facts.effects)
        .filter_map(|effect| {
            (effect.path.root
                == tyrano_js_analysis::VariableRoot::Engine(
                    tyrano_js_analysis::EngineNamespace::Mp,
                ))
            .then(|| match effect.path.segments.first() {
                Some(tyrano_js_analysis::PathSegment::Static(name)) => Some(name.clone()),
                _ => None,
            })
            .flatten()
        })
        .collect::<Vec<_>>();
    dependencies.sort();
    dependencies.dedup();
    dependencies
}

fn conservative_targets(db: &dyn ProjectDb, kind: FlowEdgeKind) -> Vec<UnitKey> {
    let project = db.project();
    let (files, unit_kind): (&[File], FlowUnitKind) = match kind {
        FlowEdgeKind::Jump | FlowEdgeKind::Call | FlowEdgeKind::Event => {
            (project.scenario_files(db), FlowUnitKind::FileRoot)
        }
        FlowEdgeKind::Plugin => (project.library_files(db), FlowUnitKind::FileRoot),
        FlowEdgeKind::LoadJavaScript => (project.javascript_files(db), FlowUnitKind::JavaScript),
        FlowEdgeKind::MacroCall | FlowEdgeKind::MacroParameterForward => return Vec::new(),
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

    #[test]
    fn resolves_dynamic_storage_candidates_from_javascript_values() {
        let db = ProjectBuilder::new()
            .file(
                "data/scenario/first.ks",
                "[iscript]\ntf.next = 'menu.ks';\n[endscript]\n[jump storage=\"&tf.next\"]\n",
            )
            .file("data/scenario/menu.ks", "*start\n")
            .file("data/scenario/unrelated.ks", "*start\n")
            .build();
        let flow = analyze_project_flow(&db);
        let dynamic_targets = flow
            .edges
            .iter()
            .filter(|edge| edge.dynamic)
            .collect::<Vec<_>>();
        assert_eq!(dynamic_targets.len(), 1);
        let target = dynamic_targets[0].to.expect("resolved dynamic target");
        assert_eq!(
            flow.units[target].file.path(&db).as_str(),
            "data/scenario/menu.ks"
        );
    }

    #[test]
    fn macro_star_creates_parameter_forwarding_edge() {
        let db = ProjectBuilder::new()
            .file(
                "data/scenario/first.ks",
                "[macro name=inner][endmacro]\n[macro name=outer][inner *][endmacro]\n[outer value=1]\n",
            )
            .build();
        let flow = analyze_project_flow(&db);
        assert!(
            flow.edges.iter().any(|edge| {
                edge.kind == FlowEdgeKind::MacroParameterForward && edge.to.is_some()
            })
        );
    }

    #[test]
    fn mp_expressions_and_star_resolve_macro_body_storage_edges() {
        let db = ProjectBuilder::new()
            .file(
                "data/scenario/first.ks",
                "[macro name=go][jump storage=\"&mp.file + '.ks'\"][endmacro]\n[macro name=forward][go *][endmacro]\n[forward file=menu]\n",
            )
            .file("data/scenario/menu.ks", "*start\n")
            .file("data/scenario/unrelated.ks", "*start\n")
            .build();
        let bindings = analyze_macro_value_bindings(&db);
        assert_eq!(
            bindings.by_macro["go"]["file"]
                .string_values()
                .collect::<Vec<_>>(),
            ["menu"]
        );
        let flow = analyze_project_flow(&db);
        assert!(flow.edges.iter().any(|edge| {
            edge.dynamic
                && edge.to.is_some_and(|target| {
                    flow.units[target].file.path(&db).as_str() == "data/scenario/menu.ks"
                })
        }));
    }
}
