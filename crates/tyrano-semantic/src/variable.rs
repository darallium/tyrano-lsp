//! Project-wide abstract interpretation of engine variable lifecycles.
//!
//! Tyrano's `f/sf/tf/mp` values are shared JavaScript object properties, so
//! the base domain is definition state plus dereference/execute checks. Rust-
//! like exclusive ownership would be unsound unless a particular runtime API
//! explicitly guarantees it.

use std::collections::{HashMap, HashSet, VecDeque};

use tyrano_js_analysis::{
    EffectKind, EffectRole, EngineNamespace, JsEffect, PathSegment, VariableRoot,
};
use tyrano_project::{File, ProjectDb};
use tyrano_syntax::text::{TextRange, TextSize};

use crate::flow::macro_at;
use crate::{
    FlowEdgeKind, FlowUnit, FlowUnitKind, ProjectFlowAnalysis, analyze_project_flow,
    file_js_analysis,
};

#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum StateSegment {
    Static(String),
    Dynamic,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct StatePath {
    pub namespace: EngineNamespace,
    pub segments: Vec<StateSegment>,
}

impl StatePath {
    pub fn render(&self) -> String {
        let mut out = match self.namespace {
            EngineNamespace::F => "f",
            EngineNamespace::Sf => "sf",
            EngineNamespace::Tf => "tf",
            EngineNamespace::Mp => "mp",
        }
        .to_owned();
        for segment in &self.segments {
            match segment {
                StateSegment::Static(name) => {
                    out.push('.');
                    out.push_str(name);
                }
                StateSegment::Dynamic => out.push_str("[*]"),
            }
        }
        out
    }

    fn parent(&self) -> Option<StatePath> {
        (!self.segments.is_empty()).then(|| StatePath {
            namespace: self.namespace,
            segments: self.segments[..self.segments.len() - 1].to_vec(),
        })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AbstractDefinition {
    Undefined,
    Defined,
    MaybeDefined,
}

impl AbstractDefinition {
    fn join(self, other: AbstractDefinition) -> AbstractDefinition {
        if self == other {
            self
        } else {
            AbstractDefinition::MaybeDefined
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum LifecycleIssueKind {
    UndefinedDereference,
    MaybeUndefinedDereference,
    UndefinedExecute,
    MaybeUndefinedExecute,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LifecycleIssue {
    pub kind: LifecycleIssueKind,
    pub unit: usize,
    pub path: StatePath,
    pub file: File,
    pub range: TextRange,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum DerivedDefinitionKind {
    Define,
    MaybeDefine,
}

/// A property creation derived from incoming state and a successful Write.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DerivedDefinition {
    pub kind: DerivedDefinitionKind,
    pub unit: usize,
    pub path: StatePath,
    pub file: File,
    pub range: TextRange,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ProjectVariableAnalysis {
    pub flow: ProjectFlowAnalysis,
    pub paths: Vec<StatePath>,
    pub issues: Vec<LifecycleIssue>,
    pub definitions: Vec<DerivedDefinition>,
    pub iterations: usize,
}

impl ProjectVariableAnalysis {
    pub fn definite_issue_count(&self) -> usize {
        self.issues
            .iter()
            .filter(|issue| {
                matches!(
                    issue.kind,
                    LifecycleIssueKind::UndefinedDereference | LifecycleIssueKind::UndefinedExecute
                )
            })
            .count()
    }

    pub fn possible_issue_count(&self) -> usize {
        self.issues.len() - self.definite_issue_count()
    }
}

#[derive(Clone)]
struct LocatedEffect {
    effect: JsEffect,
    range: TextRange,
    order: u32,
}

struct TransferRecorder<'a> {
    path_ids: &'a HashMap<StatePath, usize>,
    record: bool,
    issues: &'a mut Vec<LifecycleIssue>,
    issue_keys: &'a mut HashSet<(usize, LifecycleIssueKind, StatePath, u32, u32)>,
    definitions: &'a mut Vec<DerivedDefinition>,
    definition_keys: &'a mut HashSet<(usize, DerivedDefinitionKind, StatePath, u32, u32)>,
}

pub fn analyze_project_variables(db: &dyn ProjectDb) -> ProjectVariableAnalysis {
    let flow = analyze_project_flow(db);
    let Some(entry) = flow.entry else {
        return ProjectVariableAnalysis {
            flow,
            ..ProjectVariableAnalysis::default()
        };
    };
    let effects: Vec<Vec<LocatedEffect>> = flow
        .units
        .iter()
        .map(|unit| effects_for_unit(db, unit))
        .collect();
    let mut paths: Vec<StatePath> = effects
        .iter()
        .flatten()
        .filter_map(|located| state_path(&located.effect))
        .filter(|path| !path.segments.is_empty())
        .collect();
    paths.sort();
    paths.dedup();
    let path_ids: HashMap<StatePath, usize> = paths
        .iter()
        .cloned()
        .enumerate()
        .map(|(id, path)| (path, id))
        .collect();

    let unit_count = flow.units.len();
    let mut inputs = vec![vec![AbstractDefinition::Undefined; paths.len()]; unit_count];
    let mut input_seen = vec![false; unit_count];
    let mut outputs = inputs.clone();
    let mut output_seen = vec![false; unit_count];
    input_seen[entry] = true;
    let mut queue = VecDeque::from([entry]);
    let mut queued = vec![false; unit_count];
    queued[entry] = true;
    let mut issues = Vec::new();
    let mut issue_keys = HashSet::new();
    let mut definitions = Vec::new();
    let mut definition_keys = HashSet::new();
    let mut iterations = 0;

    while let Some(unit_id) = queue.pop_front() {
        queued[unit_id] = false;
        iterations += 1;
        let output = transfer_unit(
            unit_id,
            &flow.units[unit_id],
            &inputs[unit_id],
            &effects[unit_id],
            &mut TransferRecorder {
                path_ids: &path_ids,
                record: false,
                issues: &mut issues,
                issue_keys: &mut issue_keys,
                definitions: &mut definitions,
                definition_keys: &mut definition_keys,
            },
        );
        let output_changed = !output_seen[unit_id] || outputs[unit_id] != output;
        outputs[unit_id] = output;
        output_seen[unit_id] = true;
        if !output_changed {
            continue;
        }
        let mut targets: Vec<usize> = flow
            .edges
            .iter()
            .filter(|edge| edge.from == unit_id)
            .filter_map(|edge| edge.to)
            .collect();
        // Calls return to their continuation. At unit granularity the exact
        // continuation is joined at the caller entry, yielding MaybeDefined
        // rather than unsafely treating callee writes as preceding all code.
        targets.extend(flow.edges.iter().filter_map(|edge| {
            (edge.to == Some(unit_id)
                && matches!(
                    edge.kind,
                    FlowEdgeKind::Call
                        | FlowEdgeKind::MacroCall
                        | FlowEdgeKind::Plugin
                        | FlowEdgeKind::LoadJavaScript
                ))
            .then_some(edge.from)
        }));
        targets.sort_unstable();
        targets.dedup();
        for target in targets {
            let changed = if input_seen[target] {
                join_into(&mut inputs[target], &outputs[unit_id])
            } else {
                inputs[target].clone_from(&outputs[unit_id]);
                input_seen[target] = true;
                true
            };
            if changed && !queued[target] {
                queue.push_back(target);
                queued[target] = true;
            }
        }
    }

    // Diagnostics and derived definitions must be based on the converged
    // inputs. Recording during the worklist would retain stale provisional
    // Undefined results after a later join changed them to Maybe/Defined.
    for unit_id in 0..unit_count {
        if !input_seen[unit_id] {
            continue;
        }
        let _ = transfer_unit(
            unit_id,
            &flow.units[unit_id],
            &inputs[unit_id],
            &effects[unit_id],
            &mut TransferRecorder {
                path_ids: &path_ids,
                record: true,
                issues: &mut issues,
                issue_keys: &mut issue_keys,
                definitions: &mut definitions,
                definition_keys: &mut definition_keys,
            },
        );
    }

    issues.sort_by_key(|issue| {
        (
            issue.unit,
            issue.range.start(),
            issue.range.end(),
            issue.path.clone(),
        )
    });
    definitions.sort_by_key(|definition| {
        (
            definition.unit,
            definition.range.start(),
            definition.range.end(),
            definition.path.clone(),
        )
    });
    ProjectVariableAnalysis {
        flow,
        paths,
        issues,
        definitions,
        iterations,
    }
}

fn join_into(input: &mut [AbstractDefinition], incoming: &[AbstractDefinition]) -> bool {
    let mut changed = false;
    for (slot, incoming) in input.iter_mut().zip(incoming) {
        let joined = slot.join(*incoming);
        if *slot != joined {
            *slot = joined;
            changed = true;
        }
    }
    changed
}

fn transfer_unit(
    unit_id: usize,
    unit: &FlowUnit,
    input: &[AbstractDefinition],
    effects: &[LocatedEffect],
    recorder: &mut TransferRecorder<'_>,
) -> Vec<AbstractDefinition> {
    let mut state = input.to_vec();
    if matches!(unit.kind, FlowUnitKind::Macro(_)) {
        for (path, &id) in recorder.path_ids {
            if path.namespace == EngineNamespace::Mp && state[id] == AbstractDefinition::Undefined {
                // Macro parameters are caller-provided and untyped: absence is
                // possible, but they are not definitely absent.
                state[id] = AbstractDefinition::MaybeDefined;
            }
        }
    }
    for located in effects {
        let Some(path) = state_path(&located.effect) else {
            continue;
        };
        if path.segments.is_empty() {
            continue;
        }
        let definition = recorder
            .path_ids
            .get(&path)
            .map_or(AbstractDefinition::Undefined, |&id| state[id]);
        let optional = located.effect.deferred || located.effect.conditional;
        let checked_definition = if optional
            && definition == AbstractDefinition::Undefined
        {
            AbstractDefinition::MaybeDefined
        } else {
            definition
        };
        let issue_kind = match (
            located.effect.kind,
            located.effect.role,
            checked_definition,
        ) {
            (EffectKind::Read, EffectRole::Dereference, AbstractDefinition::Undefined) => {
                Some(LifecycleIssueKind::UndefinedDereference)
            }
            (EffectKind::Read, EffectRole::Dereference, AbstractDefinition::MaybeDefined) => {
                Some(LifecycleIssueKind::MaybeUndefinedDereference)
            }
            (EffectKind::Execute, _, AbstractDefinition::Undefined) => {
                Some(LifecycleIssueKind::UndefinedExecute)
            }
            (EffectKind::Execute, _, AbstractDefinition::MaybeDefined) => {
                Some(LifecycleIssueKind::MaybeUndefinedExecute)
            }
            _ => None,
        };
        if recorder.record && let Some(kind) = issue_kind {
            let key = (
                unit_id,
                kind,
                path.clone(),
                u32::from(located.range.start()),
                u32::from(located.range.end()),
            );
            if recorder.issue_keys.insert(key) {
                recorder.issues.push(LifecycleIssue {
                    kind,
                    unit: unit_id,
                    path: path.clone(),
                    file: unit.file,
                    range: located.range,
                });
            }
        }
        if located.effect.kind == EffectKind::Write {
            let imprecise = path.segments.contains(&StateSegment::Dynamic);
            if located.effect.role == EffectRole::Delete {
                if let Some(&id) = recorder.path_ids.get(&path) {
                    state[id] = if optional || imprecise {
                        state[id].join(AbstractDefinition::Undefined)
                    } else {
                        AbstractDefinition::Undefined
                    };
                }
                continue;
            }
            let parent_state = path.parent().map_or(AbstractDefinition::Defined, |parent| {
                if parent.segments.is_empty() {
                    AbstractDefinition::Defined
                } else {
                    recorder
                        .path_ids
                        .get(&parent)
                        .map_or(AbstractDefinition::Undefined, |&id| state[id])
                }
            });
            let derived = match (definition, parent_state) {
                (AbstractDefinition::Undefined, AbstractDefinition::Defined) => {
                    Some(DerivedDefinitionKind::Define)
                }
                (
                    AbstractDefinition::Undefined | AbstractDefinition::MaybeDefined,
                    AbstractDefinition::MaybeDefined,
                )
                | (AbstractDefinition::MaybeDefined, AbstractDefinition::Defined) => {
                    Some(DerivedDefinitionKind::MaybeDefine)
                }
                _ => None,
            };
            if recorder.record && let Some(kind) = derived {
                let kind = if optional || imprecise {
                    DerivedDefinitionKind::MaybeDefine
                } else {
                    kind
                };
                let key = (
                    unit_id,
                    kind,
                    path.clone(),
                    u32::from(located.range.start()),
                    u32::from(located.range.end()),
                );
                if recorder.definition_keys.insert(key) {
                    recorder.definitions.push(DerivedDefinition {
                        kind,
                        unit: unit_id,
                        path: path.clone(),
                        file: unit.file,
                        range: located.range,
                    });
                }
            }
            if let Some(&id) = recorder.path_ids.get(&path) {
                let next = if imprecise && parent_state == AbstractDefinition::Defined {
                    AbstractDefinition::MaybeDefined
                } else {
                    parent_state
                };
                state[id] = if optional {
                    state[id].join(next)
                } else {
                    next
                };
            }
        }
    }
    state
}

fn effects_for_unit(db: &dyn ProjectDb, unit: &FlowUnit) -> Vec<LocatedEffect> {
    let analysis = file_js_analysis(db, unit.file);
    let multi_entry = matches!(unit.kind, FlowUnitKind::FileRoot)
        && tyrano_db::parsed_module(db, unit.file.source(db))
            .scenario()
            .lines()
            .any(|line| matches!(line, tyrano_syntax::ast::Line::Label(_)));
    let mut out = Vec::new();
    for region in &analysis.regions {
        let belongs = match &unit.kind {
            FlowUnitKind::JavaScript => true,
            FlowUnitKind::FileRoot => macro_at(db, unit.file, region.range).is_none(),
            FlowUnitKind::Macro(name) => {
                macro_at(db, unit.file, region.range).as_deref() == Some(name)
            }
        };
        if !belongs {
            continue;
        }
        let anchor = u32::from(region.range.start());
        for effect in &region.facts.effects {
            let mut effect = effect.clone();
            effect.conditional |= multi_entry;
            let order = anchor + effect.order;
            let range = TextRange::new(
                TextSize::new(anchor + effect.range.start),
                TextSize::new(anchor + effect.range.end),
            );
            out.push(LocatedEffect {
                effect,
                range,
                order,
            });
        }
    }
    // Root code runs at load/eval time. Function bodies are possible future
    // executions, so apply them afterwards as optional joins regardless of
    // their declaration position in the source.
    out.sort_by_key(|effect| (effect.effect.deferred, effect.order));
    out
}

fn state_path(effect: &JsEffect) -> Option<StatePath> {
    let VariableRoot::Engine(namespace) = effect.path.root else {
        return None;
    };
    Some(StatePath {
        namespace,
        segments: effect
            .path
            .segments
            .iter()
            .map(|segment| match segment {
                PathSegment::Static(name) => StateSegment::Static(name.clone()),
                PathSegment::Dynamic(_) => StateSegment::Dynamic,
            })
            .collect(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use tyrano_project::testing::ProjectBuilder;

    #[test]
    fn undefined_intermediate_path_is_not_defined_by_terminal_write() {
        let db = ProjectBuilder::new()
            .file(
                "data/scenario/first.ks",
                "[iscript]\nf.player.hp = 100;\n[endscript]\n",
            )
            .build();
        let analysis = analyze_project_variables(&db);
        assert!(analysis.issues.iter().any(|issue| {
            issue.kind == LifecycleIssueKind::UndefinedDereference
                && issue.path.render() == "f.player"
        }));
        assert!(
            !analysis
                .definitions
                .iter()
                .any(|definition| definition.path.render() == "f.player.hp")
        );
    }

    #[test]
    fn explicit_parent_initialization_makes_nested_write_safe() {
        let db = ProjectBuilder::new()
            .file(
                "data/scenario/first.ks",
                "[iscript]\nf.player = {}; f.player.hp = 100;\n[endscript]\n",
            )
            .build();
        let analysis = analyze_project_variables(&db);
        assert!(!analysis.issues.iter().any(|issue| {
            issue.path.render() == "f.player"
                && matches!(
                    issue.kind,
                    LifecycleIssueKind::UndefinedDereference
                        | LifecycleIssueKind::MaybeUndefinedDereference
                )
        }));
        assert!(analysis.definitions.iter().any(|definition| {
            definition.kind == DerivedDefinitionKind::Define
                && definition.path.render() == "f.player.hp"
        }));
    }

    #[test]
    fn delete_returns_a_path_to_undefined() {
        let db = ProjectBuilder::new()
            .file(
                "data/scenario/first.ks",
                "[iscript]\nf.player = {}; delete f.player; f.player.hp = 100;\n[endscript]\n",
            )
            .build();
        let analysis = analyze_project_variables(&db);

        assert!(analysis.issues.iter().any(|issue| {
            issue.kind == LifecycleIssueKind::UndefinedDereference
                && issue.path.render() == "f.player"
        }));
        assert!(!analysis
            .definitions
            .iter()
            .any(|definition| definition.path.render() == "f.player.hp"));
    }

    #[test]
    fn uncalled_function_write_does_not_define_root_flow() {
        let db = ProjectBuilder::new()
            .file(
                "data/scenario/first.ks",
                "[iscript]\nfunction setup() { f.player = {}; }\nf.player.hp = 100;\n[endscript]\n",
            )
            .build();
        let analysis = analyze_project_variables(&db);

        assert!(analysis.issues.iter().any(|issue| {
            issue.kind == LifecycleIssueKind::UndefinedDereference
                && issue.path.render() == "f.player"
        }));
        assert!(!analysis.definitions.iter().any(|definition| {
            definition.kind == DerivedDefinitionKind::Define
                && definition.path.render() == "f.player"
        }));
    }

    #[test]
    fn dynamic_key_write_does_not_define_every_possible_key() {
        let db = ProjectBuilder::new()
            .file(
                "data/scenario/first.ks",
                "[iscript]\nf.players = {}; f.players[key] = {}; f.players[other].hp = 100;\n[endscript]\n",
            )
            .build();
        let analysis = analyze_project_variables(&db);

        assert!(analysis.issues.iter().any(|issue| {
            issue.kind == LifecycleIssueKind::MaybeUndefinedDereference
                && issue.path.render() == "f.players[*]"
        }));
        assert!(!analysis.definitions.iter().any(|definition| {
            definition.kind == DerivedDefinitionKind::Define
                && definition.path.render() == "f.players[*]"
        }));
    }

    #[test]
    fn conditional_write_is_only_maybe_defined() {
        let db = ProjectBuilder::new()
            .file(
                "data/scenario/first.ks",
                "[iscript]\nif (ready) f.player = {};\nf.player.hp = 100;\n[endscript]\n",
            )
            .build();
        let analysis = analyze_project_variables(&db);

        assert!(analysis.issues.iter().any(|issue| {
            issue.kind == LifecycleIssueKind::MaybeUndefinedDereference
                && issue.path.render() == "f.player"
        }));
        assert!(!analysis.definitions.iter().any(|definition| {
            definition.kind == DerivedDefinitionKind::Define
                && definition.path.render() == "f.player"
        }));
    }

    #[test]
    fn label_reentry_cannot_reuse_a_pre_label_definition() {
        let db = ProjectBuilder::new()
            .file(
                "data/scenario/first.ks",
                "[eval exp='f.player = {}']\n*again\n[eval exp='f.player.hp = 100']\n[jump target=*again]\n",
            )
            .build();
        let analysis = analyze_project_variables(&db);

        assert!(analysis.issues.iter().any(|issue| {
            issue.kind == LifecycleIssueKind::MaybeUndefinedDereference
                && issue.path.render() == "f.player"
        }));
        assert!(!analysis.definitions.iter().any(|definition| {
            definition.kind == DerivedDefinitionKind::Define
                && definition.path.render() == "f.player"
        }));
    }

    #[test]
    fn event_join_produces_maybe_defined_without_interleaving_enumeration() {
        let db = ProjectBuilder::new()
            .file(
                "data/scenario/first.ks",
                "[button storage=a.ks target=*go]\n[button storage=b.ks target=*go]\n[s]\n",
            )
            .file(
                "data/scenario/a.ks",
                "*go\n[eval exp=\"f.player = {}\"]\n[jump storage=c.ks]\n",
            )
            .file("data/scenario/b.ks", "*go\n[jump storage=c.ks]\n")
            .file("data/scenario/c.ks", "[eval exp=\"f.player.hp = 1\"]\n")
            .build();
        let analysis = analyze_project_variables(&db);
        assert!(analysis.issues.iter().any(|issue| {
            issue.kind == LifecycleIssueKind::MaybeUndefinedDereference
                && issue.path.render() == "f.player"
        }));
        assert!(!analysis.issues.iter().any(|issue| {
            issue.kind == LifecycleIssueKind::UndefinedDereference
                && issue.path.render() == "f.player"
        }));
        assert!(analysis.iterations < 20);
    }
}
