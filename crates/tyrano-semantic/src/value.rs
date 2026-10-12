//! Project-hosted abstract JavaScript values.
//!
//! The Oxc layer preserves expression structure. This layer evaluates that
//! structure in scenario order, so an entity such as
//! `&tf.base + 'button.png'` has finite storage candidates when preceding
//! script assigned `tf.base`.

use std::collections::{BTreeMap, BTreeSet, HashMap};

use tyrano_js_analysis::{JsTypeSet, JsValueExpr, PathSegment, VariablePath, VariableRoot};
use tyrano_project::{File, ProjectDb};
use tyrano_syntax::text::TextRange;

use crate::{JsRegionKind, file_js_analysis};

const MAX_CANDIDATES: usize = 64;

/// One exact scalar JavaScript value candidate.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum ScalarValue {
    Undefined,
    Null,
    Boolean(bool),
    Number(String),
    String(String),
}

/// Finite values and type information for one JavaScript expression.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ValueCandidates {
    pub types: JsTypeSet,
    pub values: BTreeSet<ScalarValue>,
    /// Some execution path or operation could not be represented finitely.
    pub unknown: bool,
}

impl ValueCandidates {
    pub fn string(value: impl Into<String>) -> Self {
        Self {
            types: JsTypeSet::STRING,
            values: BTreeSet::from([ScalarValue::String(value.into())]),
            unknown: false,
        }
    }

    pub fn join(&mut self, other: &ValueCandidates) {
        self.types = self.types.union(other.types);
        self.unknown |= other.unknown;
        for value in &other.values {
            if self.values.len() == MAX_CANDIDATES {
                self.unknown = true;
                break;
            }
            self.values.insert(value.clone());
        }
    }

    pub fn string_values(&self) -> impl Iterator<Item = &str> {
        self.values.iter().filter_map(|value| match value {
            ScalarValue::String(value) => Some(value.as_str()),
            _ => None,
        })
    }

    pub fn is_definitely_finite_strings(&self) -> bool {
        !self.unknown
            && !self.values.is_empty()
            && self
                .values
                .iter()
                .all(|value| matches!(value, ScalarValue::String(_)))
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct AbstractValue {
    candidates: ValueCandidates,
    object: Option<BTreeMap<String, AbstractValue>>,
    array: Option<Vec<AbstractValue>>,
}

impl AbstractValue {
    fn unknown() -> Self {
        Self::typed(JsTypeSet::UNKNOWN, true)
    }

    fn typed(types: JsTypeSet, unknown: bool) -> Self {
        Self {
            candidates: ValueCandidates {
                types,
                values: BTreeSet::new(),
                unknown,
            },
            object: None,
            array: None,
        }
    }

    fn scalar(value: ScalarValue, types: JsTypeSet) -> Self {
        Self {
            candidates: ValueCandidates {
                types,
                values: BTreeSet::from([value]),
                unknown: false,
            },
            object: None,
            array: None,
        }
    }

    fn from_candidates(candidates: ValueCandidates) -> Self {
        Self {
            candidates,
            object: None,
            array: None,
        }
    }

    fn join(mut self, other: Self) -> Self {
        self.candidates.types = self.candidates.types.union(other.candidates.types);
        self.candidates.unknown |= other.candidates.unknown;
        for value in other.candidates.values {
            if self.candidates.values.len() == MAX_CANDIDATES {
                self.candidates.unknown = true;
                break;
            }
            self.candidates.values.insert(value);
        }
        self.object = match (self.object.take(), other.object) {
            (Some(mut left), Some(right)) => {
                for (name, right) in right {
                    left.entry(name)
                        .and_modify(|left| *left = left.clone().join(right.clone()))
                        .or_insert(right);
                }
                Some(left)
            }
            _ => None,
        };
        self.array = None;
        self
    }
}

type ValueState = HashMap<VariablePath, AbstractValue>;

/// Evaluates the JavaScript entity/expression region contained by `host`.
/// Assignments from preceding regions in the same source are applied in
/// execution order. Lexical bindings are region-local; Tyrano namespaces
/// persist between regions.
pub fn value_candidates_in_range(
    db: &dyn ProjectDb,
    file: File,
    host: TextRange,
) -> Option<ValueCandidates> {
    let analysis = file_js_analysis(db, file);
    let target = analysis.regions.iter().position(|region| {
        host.start() <= region.range.start() && host.end() >= region.range.end()
    })?;
    evaluate_region(&analysis, target, &BTreeMap::new())
}

/// Evaluates the JavaScript region for one named tag parameter. This avoids
/// ambiguity when a tag contains several entities (for example dynamic
/// `storage=` and `target=` together).
pub fn value_candidates_for_parameter(
    db: &dyn ProjectDb,
    file: File,
    tag_range: TextRange,
    parameter: &str,
) -> Option<ValueCandidates> {
    let analysis = file_js_analysis(db, file);
    let target = analysis.regions.iter().position(|region| {
        tag_range.start() <= region.range.start()
            && tag_range.end() >= region.range.end()
            && matches!(
                &region.kind,
                JsRegionKind::Parameter { parameter: name, .. }
                    | JsRegionKind::Entity { parameter: name, .. }
                    if name == parameter
            )
    })?;
    evaluate_region(&analysis, target, &BTreeMap::new())
}

/// Evaluates a parameter with the `mp` values transferred by incoming macro
/// dependency-graph edges.
pub fn value_candidates_for_parameter_with_mp(
    db: &dyn ProjectDb,
    file: File,
    tag_range: TextRange,
    parameter: &str,
    mp: &BTreeMap<String, ValueCandidates>,
) -> Option<ValueCandidates> {
    let analysis = file_js_analysis(db, file);
    let target = analysis.regions.iter().position(|region| {
        tag_range.start() <= region.range.start()
            && tag_range.end() >= region.range.end()
            && matches!(
                &region.kind,
                JsRegionKind::Parameter { parameter: name, .. }
                    | JsRegionKind::Entity { parameter: name, .. }
                    if name == parameter
            )
    })?;
    evaluate_region(&analysis, target, mp)
}

fn evaluate_region(
    analysis: &crate::FileJsAnalysis,
    target: usize,
    mp: &BTreeMap<String, ValueCandidates>,
) -> Option<ValueCandidates> {
    let mut state = ValueState::new();
    for (name, candidates) in mp {
        state.insert(
            VariablePath {
                root: VariableRoot::Engine(tyrano_js_analysis::EngineNamespace::Mp),
                segments: vec![PathSegment::Static(name.clone())],
            },
            AbstractValue::from_candidates(candidates.clone()),
        );
    }

    for (index, region) in analysis.regions.iter().enumerate().take(target + 1) {
        state.retain(|path, _| !matches!(path.root, VariableRoot::Lexical { .. }));
        for assignment in &region.facts.assignments {
            if assignment.deferred {
                continue;
            }
            let value = evaluate(&assignment.value, &state, 0);
            if assignment.conditional {
                let previous = state
                    .get(&assignment.target)
                    .cloned()
                    .unwrap_or_else(AbstractValue::unknown);
                state.insert(assignment.target.clone(), previous.join(value));
            } else {
                state.insert(assignment.target.clone(), value);
            }
        }
        if index == target {
            return region
                .facts
                .result
                .as_ref()
                .map(|result| evaluate(result, &state, 0).candidates);
        }
    }
    None
}

fn evaluate(expression: &JsValueExpr, state: &ValueState, depth: usize) -> AbstractValue {
    if depth > 32 {
        return AbstractValue::unknown();
    }
    match expression {
        JsValueExpr::Unknown => AbstractValue::unknown(),
        JsValueExpr::Undefined => {
            AbstractValue::scalar(ScalarValue::Undefined, JsTypeSet::UNDEFINED)
        }
        JsValueExpr::Null => AbstractValue::scalar(ScalarValue::Null, JsTypeSet::NULL),
        JsValueExpr::Boolean(value) => {
            AbstractValue::scalar(ScalarValue::Boolean(*value), JsTypeSet::BOOLEAN)
        }
        JsValueExpr::Number(value) => {
            AbstractValue::scalar(ScalarValue::Number(value.clone()), JsTypeSet::NUMBER)
        }
        JsValueExpr::String(value) => {
            AbstractValue::scalar(ScalarValue::String(value.clone()), JsTypeSet::STRING)
        }
        JsValueExpr::Path(path) => {
            evaluate_path(path, state).unwrap_or_else(AbstractValue::unknown)
        }
        JsValueExpr::Add(left, right) => add(
            evaluate(left, state, depth + 1),
            evaluate(right, state, depth + 1),
        ),
        JsValueExpr::Choice(values) => values
            .iter()
            .map(|value| evaluate(value, state, depth + 1))
            .reduce(AbstractValue::join)
            .unwrap_or_else(AbstractValue::unknown),
        JsValueExpr::Array(values) => {
            let values = values
                .iter()
                .map(|value| evaluate(value, state, depth + 1))
                .collect();
            let mut result = AbstractValue::typed(JsTypeSet::ARRAY, false);
            result.array = Some(values);
            result
        }
        JsValueExpr::Object(properties) => {
            let properties = properties
                .iter()
                .map(|(name, value)| (name.clone(), evaluate(value, state, depth + 1)))
                .collect();
            let mut result = AbstractValue::typed(JsTypeSet::OBJECT, false);
            result.object = Some(properties);
            result
        }
        JsValueExpr::Function => AbstractValue::typed(JsTypeSet::FUNCTION, false),
        JsValueExpr::Typed(types) => AbstractValue::typed(*types, true),
    }
}

fn evaluate_path(path: &VariablePath, state: &ValueState) -> Option<AbstractValue> {
    if let Some(value) = state.get(path) {
        return Some(value.clone());
    }
    for prefix_len in (0..path.segments.len()).rev() {
        let prefix = VariablePath {
            root: path.root.clone(),
            segments: path.segments[..prefix_len].to_vec(),
        };
        let Some(mut value) = state.get(&prefix).cloned() else {
            continue;
        };
        for segment in &path.segments[prefix_len..] {
            let PathSegment::Static(name) = segment else {
                return None;
            };
            value = value.object.as_ref()?.get(name)?.clone();
        }
        return Some(value);
    }
    None
}

fn add(left: AbstractValue, right: AbstractValue) -> AbstractValue {
    let mut values = BTreeSet::new();
    let mut unknown = left.candidates.unknown || right.candidates.unknown;
    let string_mode = left.candidates.types.intersects(JsTypeSet::STRING)
        || right.candidates.types.intersects(JsTypeSet::STRING);
    for left in &left.candidates.values {
        for right in &right.candidates.values {
            if values.len() == MAX_CANDIDATES {
                unknown = true;
                break;
            }
            if string_mode {
                values.insert(ScalarValue::String(format!(
                    "{}{}",
                    coerce_string(left),
                    coerce_string(right)
                )));
            } else if let (ScalarValue::Number(left), ScalarValue::Number(right)) = (left, right) {
                match (left.parse::<f64>(), right.parse::<f64>()) {
                    (Ok(left), Ok(right)) => {
                        values.insert(ScalarValue::Number((left + right).to_string()));
                    }
                    _ => unknown = true,
                }
            } else {
                unknown = true;
            }
        }
    }
    let types = if string_mode {
        JsTypeSet::STRING
    } else {
        JsTypeSet::NUMBER
    };
    AbstractValue {
        candidates: ValueCandidates {
            types,
            values,
            unknown,
        },
        object: None,
        array: None,
    }
}

fn coerce_string(value: &ScalarValue) -> String {
    match value {
        ScalarValue::Undefined => "undefined".into(),
        ScalarValue::Null => "null".into(),
        ScalarValue::Boolean(value) => value.to_string(),
        ScalarValue::Number(value) | ScalarValue::String(value) => value.clone(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tyrano_project::{ProjectPath, testing::ProjectBuilder};
    use tyrano_syntax::ast::{AstNode as _, Line};

    #[test]
    fn evaluates_entity_from_preceding_script_assignments() {
        let db = ProjectBuilder::new().file(
            "data/scenario/a.ks",
            "[iscript]\ntf.base = 'config/';\n[endscript]\n[image storage=\"&tf.base + 'button.png'\"]\n",
        ).build();
        let file = db
            .file(&ProjectPath::new("data/scenario/a.ks").unwrap())
            .unwrap();
        let module = tyrano_db::parsed_module(&db, file.source(&db));
        let range = module
            .scenario()
            .lines()
            .find_map(|line| match line {
                Line::Text(text) => text
                    .syntax()
                    .descendants()
                    .find(|node| node.kind() == tyrano_syntax::SyntaxKind::PARAM)
                    .map(|node| node.text_range()),
                _ => None,
            })
            .unwrap();
        let candidates = value_candidates_in_range(&db, file, range).unwrap();
        assert_eq!(
            candidates.string_values().collect::<Vec<_>>(),
            ["config/button.png"]
        );
        assert!(!candidates.unknown);
    }
}
