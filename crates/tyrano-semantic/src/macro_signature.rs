//! Macro parameter signatures inferred from executable macro bodies.

use std::collections::BTreeMap;
use std::sync::Arc;

use tyrano_js_analysis::{EngineNamespace, JsTypeSet, PathSegment, VariableRoot};
use tyrano_project::registry::GLOBAL_PARAMS;
use tyrano_project::{ExtraParams, MacroDef, Project, ProjectDb, ValueKind};
use tyrano_syntax::ast::{AstNode as _, InterpretOptions, Param};
use tyrano_syntax::text::TextRange;

use crate::check::all_tags;
use crate::check::kind::{ValueClass, classify};
use crate::flow::macro_at;
use crate::{SemanticModel, TagResolution, file_js_analysis};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MacroParameterSpec {
    pub name: String,
    pub required: bool,
    /// Every Tyrano parameter-kind constraint imposed by the body.
    pub kinds: Vec<ValueKind>,
    /// JavaScript runtime types inferred from `mp.<name>` operations.
    pub js_types: JsTypeSet,
}

/// A statically knowable macro dispatch predicate. This is deliberately a
/// small, extensible representation of a conditional type: a forwarded `*`
/// only contributes its callee parameters when every predicate holds.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct MacroCondition {
    pub parameter: String,
    pub equals: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConditionalMacroSignature {
    pub conditions: Vec<MacroCondition>,
    pub parameters: Vec<MacroParameterSpec>,
    pub accepts_extra: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MacroSignature {
    pub definition: MacroDef,
    pub parameters: Vec<MacroParameterSpec>,
    /// Computed `mp[key]` access prevents a closed argument list.
    pub accepts_extra: bool,
    /// Parameter shapes forwarded only from conditionally selected branches.
    pub conditional: Vec<ConditionalMacroSignature>,
}

/// The definite shape of a call, plus branches whose predicates are unresolved.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SelectedMacroSignature {
    pub parameters: Vec<MacroParameterSpec>,
    pub accepts_extra: bool,
    pub matched: Vec<Vec<MacroCondition>>,
    pub pending: Vec<ConditionalMacroSignature>,
}

pub fn select_macro_signature(
    signature: &MacroSignature,
    values: &BTreeMap<String, String>,
) -> SelectedMacroSignature {
    let mut parameters = BTreeMap::new();
    for parameter in &signature.parameters {
        merge_parameter(&mut parameters, parameter);
    }
    let mut result = SelectedMacroSignature {
        parameters: Vec::new(),
        accepts_extra: signature.accepts_extra,
        matched: Vec::new(),
        pending: Vec::new(),
    };
    for branch in &signature.conditional {
        if branch
            .conditions
            .iter()
            .any(|c| values.get(&c.parameter).is_some_and(|v| v != &c.equals))
        {
            continue;
        }
        if branch
            .conditions
            .iter()
            .all(|c| values.contains_key(&c.parameter))
        {
            for parameter in &branch.parameters {
                merge_parameter(&mut parameters, parameter);
            }
            result.accepts_extra |= branch.accepts_extra;
            result.matched.push(branch.conditions.clone());
        } else {
            result.pending.push(branch.clone());
        }
    }
    result.parameters = parameters.into_values().collect();
    result
}

/// Cook only literal arguments; expressions and macro references stay unresolved.
pub fn static_macro_arguments(
    params: &[Param],
    options: &InterpretOptions,
) -> BTreeMap<String, String> {
    params
        .iter()
        .filter(|param| {
            let Some(raw) = param.raw_value() else {
                return false;
            };
            let Some(first) = raw.chars().next() else {
                return false;
            };
            !matches!(first, '\'' | '"' | '`') || (raw.len() >= 2 && raw.ends_with(first))
        })
        .filter_map(|param| match classify(param, options) {
            ValueClass::Static(value) => Some((param.name(), value)),
            _ => None,
        })
        .collect()
}

#[salsa::tracked]
pub fn project_macro_signatures(db: &dyn ProjectDb, project: Project) -> Arc<[MacroSignature]> {
    let definitions = tyrano_project::macro_registry(db, project)
        .iter()
        .map(|(_, definition)| definition.clone())
        .collect::<Vec<_>>();
    let mut current = definitions
        .iter()
        .map(|definition| MacroSignature {
            definition: definition.clone(),
            parameters: Vec::new(),
            accepts_extra: false,
            conditional: Vec::new(),
        })
        .collect::<Vec<_>>();

    // Constraints only grow. A full pass must add a parameter or a type/kind
    // bit to continue, so this bound is generous even for recursive macros.
    let bound = definitions.len().saturating_mul(32).max(1);
    for _ in 0..bound {
        let next = definitions
            .iter()
            .map(|definition| infer_signature(db, definition, &current))
            .collect::<Vec<_>>();
        if next == current {
            return next.into();
        }
        current = next;
    }
    current.into()
}

pub fn macro_signature(db: &dyn ProjectDb, definition: &MacroDef) -> Option<MacroSignature> {
    project_macro_signatures(db, db.project())
        .iter()
        .find(|signature| signature.definition == *definition)
        .cloned()
}

fn infer_signature(
    db: &dyn ProjectDb,
    definition: &MacroDef,
    current: &[MacroSignature],
) -> MacroSignature {
    let source_file = definition.file.source(db);
    let module = tyrano_db::parsed_module(db, source_file);
    let options = source_file.interpret_options(db);
    let js = file_js_analysis(db, definition.file);
    let model = SemanticModel::new(db, definition.file);
    let mut parameters: BTreeMap<String, MacroParameterSpec> = BTreeMap::new();
    let mut accepts_extra = false;
    let mut conditional = BTreeMap::<Vec<MacroCondition>, ConditionalMacroWork>::new();

    for region in &js.regions {
        if macro_at(db, definition.file, region.range).as_deref() != Some(&definition.name) {
            continue;
        }
        for effect in &region.facts.effects {
            if effect.path.root != VariableRoot::Engine(EngineNamespace::Mp) {
                continue;
            }
            match effect.path.segments.first() {
                Some(PathSegment::Static(name)) => {
                    parameter(&mut parameters, name);
                }
                Some(PathSegment::Dynamic(_)) | None => accepts_extra = true,
            }
        }
        for constraint in &region.facts.constraints {
            if constraint.path.root == VariableRoot::Engine(EngineNamespace::Mp) {
                match constraint.path.segments.first() {
                    Some(PathSegment::Static(name)) => {
                        let parameter = parameter(&mut parameters, name);
                        parameter.js_types = parameter.js_types.union(constraint.expected);
                    }
                    Some(PathSegment::Dynamic(_)) | None => accepts_extra = true,
                }
            }
        }
    }

    let mut control_flow = Vec::<Option<MacroCondition>>::new();
    for tag in all_tags(&module.scenario()) {
        let tag_range = tag.syntax().text_range();
        if macro_at(db, definition.file, tag_range).as_deref() != Some(&definition.name) {
            continue;
        }
        match tag.name().as_str() {
            "if" => {
                control_flow.push(tag_condition(&tag, &js.regions));
                continue;
            }
            "elsif" => {
                if let Some(branch) = control_flow.last_mut() {
                    *branch = tag_condition(&tag, &js.regions);
                }
                continue;
            }
            "else" => {
                if let Some(branch) = control_flow.last_mut() {
                    *branch = None;
                }
                continue;
            }
            "endif" => {
                control_flow.pop();
                continue;
            }
            _ => {}
        }
        let resolution = model.resolve_tag(&tag.name());
        let callee = match &resolution {
            TagResolution::Macro(callee) => current
                .iter()
                .find(|signature| signature.definition == *callee),
            _ => None,
        };

        if tag.params().iter().any(|param| param.is_macro_star()) {
            let conditions = control_flow.iter().flatten().cloned().collect::<Vec<_>>();
            let (target_parameters, target_accepts_extra) = if conditions.is_empty() {
                (&mut parameters, &mut accepts_extra)
            } else {
                let work = conditional.entry(conditions).or_default();
                (&mut work.parameters, &mut work.accepts_extra)
            };
            let explicit = tag
                .params()
                .into_iter()
                .filter(|param| !param.is_macro_star())
                .map(|param| param.name())
                .collect::<std::collections::HashSet<_>>();
            match &resolution {
                TagResolution::Builtin(spec) => {
                    let implementation =
                        crate::project_javascript_tag_spec(db, db.project(), &tag.name());
                    for expected in spec
                        .params
                        .iter()
                        .filter(|expected| !explicit.contains(expected.name))
                    {
                        let inferred = parameter(target_parameters, expected.name);
                        inferred.required |= expected.required;
                        push_kind(&mut inferred.kinds, expected.kind);
                        if let Some(types) = implementation.as_ref().and_then(|implementation| {
                            implementation
                                .parameters
                                .iter()
                                .find(|parameter| parameter.name == expected.name)
                                .map(|parameter| parameter.types)
                        }) {
                            inferred.js_types = inferred.js_types.union(types);
                        }
                    }
                    if let Some(implementation) = implementation {
                        for expected in implementation.parameters.iter().filter(|expected| {
                            spec.param(&expected.name).is_none()
                                && !explicit.contains(&expected.name)
                        }) {
                            let inferred = parameter(target_parameters, &expected.name);
                            inferred.required |= expected.required;
                            inferred.js_types = inferred.js_types.union(expected.types);
                        }
                    }
                    *target_accepts_extra |= spec.extra == ExtraParams::Allow;
                }
                TagResolution::Plugin(spec) => {
                    for expected in spec
                        .parameters
                        .iter()
                        .filter(|expected| !explicit.contains(&expected.name))
                    {
                        let inferred = parameter(target_parameters, &expected.name);
                        inferred.required |= expected.required;
                        inferred.js_types = inferred.js_types.union(expected.types);
                    }
                }
                TagResolution::Macro(_) => {
                    if let Some(callee) = callee {
                        for expected in &callee.parameters {
                            if !explicit.contains(&expected.name) {
                                merge_parameter(target_parameters, expected);
                            }
                        }
                        *target_accepts_extra |= callee.accepts_extra;
                    }
                }
                TagResolution::Unknown => *target_accepts_extra = true,
            }
        }

        for actual in tag.params() {
            if actual.is_macro_star() || GLOBAL_PARAMS.contains(&actual.name().as_str()) {
                continue;
            }
            let sources = source_macro_parameters(&actual, &options, &js.regions);
            if sources.is_empty() {
                continue;
            }
            let expected_kind = match &resolution {
                TagResolution::Builtin(spec) => spec.param(&actual.name()).map(|param| param.kind),
                _ => None,
            };
            let expected_js = match &resolution {
                TagResolution::Builtin(_) => {
                    crate::project_javascript_tag_spec(db, db.project(), &tag.name())
                        .and_then(|spec| {
                            spec.parameters
                                .into_iter()
                                .find(|parameter| parameter.name == actual.name())
                        })
                        .map(|parameter| parameter.types)
                }
                TagResolution::Plugin(spec) => spec
                    .parameters
                    .iter()
                    .find(|parameter| parameter.name == actual.name())
                    .map(|parameter| parameter.types),
                TagResolution::Macro(_) => callee
                    .and_then(|callee| {
                        callee
                            .parameters
                            .iter()
                            .find(|parameter| parameter.name == actual.name())
                    })
                    .map(|parameter| parameter.js_types),
                _ => None,
            };
            let expected_macro = callee.and_then(|callee| {
                callee
                    .parameters
                    .iter()
                    .find(|parameter| parameter.name == actual.name())
            });
            for (source, required) in sources {
                let parameter = parameter(&mut parameters, &source);
                parameter.required |= required;
                if let Some(kind) = expected_kind {
                    push_kind(&mut parameter.kinds, kind);
                }
                if let Some(types) = expected_js {
                    parameter.js_types = parameter.js_types.union(types);
                }
                if let Some(expected) = expected_macro {
                    for &kind in &expected.kinds {
                        push_kind(&mut parameter.kinds, kind);
                    }
                    parameter.js_types = parameter.js_types.union(expected.js_types);
                }
            }
        }
    }

    MacroSignature {
        definition: definition.clone(),
        parameters: parameters.into_values().collect(),
        accepts_extra,
        conditional: conditional
            .into_iter()
            .map(|(conditions, work)| ConditionalMacroSignature {
                conditions,
                parameters: work.parameters.into_values().collect(),
                accepts_extra: work.accepts_extra,
            })
            .collect(),
    }
}

#[derive(Default)]
struct ConditionalMacroWork {
    parameters: BTreeMap<String, MacroParameterSpec>,
    accepts_extra: bool,
}

fn tag_condition(
    tag: &tyrano_syntax::ast::AnyTag,
    regions: &[crate::JsRegion],
) -> Option<MacroCondition> {
    let exp = tag
        .params()
        .into_iter()
        .find(|param| param.name() == "exp")?;
    let range = exp.syntax().text_range();
    let equality = regions
        .iter()
        .filter(|region| contains(range, region.range))
        .flat_map(|region| &region.facts.equalities)
        .find(|equality| {
            equality.path.root == VariableRoot::Engine(EngineNamespace::Mp)
                && matches!(equality.path.segments.as_slice(), [PathSegment::Static(_)])
        })?;
    let [PathSegment::Static(parameter)] = equality.path.segments.as_slice() else {
        return None;
    };
    Some(MacroCondition {
        parameter: parameter.clone(),
        equals: equality.value.clone(),
    })
}

fn source_macro_parameters(
    param: &Param,
    options: &InterpretOptions,
    regions: &[crate::JsRegion],
) -> Vec<(String, bool)> {
    match classify(param, options) {
        ValueClass::ParamRef => {
            let cooked = param.cooked_value(options).unwrap_or_default();
            let reference = cooked.trim_start().strip_prefix('%').unwrap_or(&cooked);
            let (name, fallback) = reference
                .split_once('|')
                .map_or((reference, None), |(name, fallback)| (name, Some(fallback)));
            (!name.is_empty())
                .then(|| (name.to_owned(), fallback.is_none()))
                .into_iter()
                .collect()
        }
        ValueClass::Entity => {
            let host = param.syntax().text_range();
            let mut names = regions
                .iter()
                .filter(|region| contains(host, region.range))
                .flat_map(|region| {
                    region.facts.effects.iter().filter_map(|effect| {
                        (effect.path.root == VariableRoot::Engine(EngineNamespace::Mp))
                            .then(|| match effect.path.segments.first() {
                                Some(PathSegment::Static(name)) => Some((name.clone(), false)),
                                _ => None,
                            })
                            .flatten()
                    })
                })
                .collect::<Vec<_>>();
            names.sort();
            names.dedup();
            names
        }
        ValueClass::Static(_) | ValueClass::Absent => Vec::new(),
    }
}

fn parameter<'a>(
    parameters: &'a mut BTreeMap<String, MacroParameterSpec>,
    name: &str,
) -> &'a mut MacroParameterSpec {
    parameters
        .entry(name.to_owned())
        .or_insert_with(|| MacroParameterSpec {
            name: name.to_owned(),
            required: false,
            kinds: Vec::new(),
            js_types: JsTypeSet::NONE,
        })
}

fn merge_parameter(
    parameters: &mut BTreeMap<String, MacroParameterSpec>,
    incoming: &MacroParameterSpec,
) {
    let target = parameter(parameters, &incoming.name);
    target.required |= incoming.required;
    for &kind in &incoming.kinds {
        push_kind(&mut target.kinds, kind);
    }
    target.js_types = target.js_types.union(incoming.js_types);
}

fn push_kind(kinds: &mut Vec<ValueKind>, kind: ValueKind) {
    if !kinds.contains(&kind) {
        kinds.push(kind);
    }
}

fn contains(outer: TextRange, inner: TextRange) -> bool {
    outer.start() <= inner.start() && outer.end() >= inner.end()
}

#[cfg(test)]
mod tests {
    use super::*;
    use tyrano_project::testing::ProjectBuilder;

    #[test]
    fn infers_percent_mp_types_and_star_forwarding_to_fixed_point() {
        let db = ProjectBuilder::new().file(
            "data/scenario/first.ks",
            "[macro name=inner]\n[wait time=%delay]\n[eval exp=\"mp.count * 2\"]\n[if exp=\"mp.enabled === true\"][endif]\n[iscript]\nmp.title.trim();\n[endscript]\n[endmacro]\n[macro name=outer][inner delay=%time count=&mp.total *][endmacro]\n[outer time=100 total=2]\n",
        ).build();
        let signatures = project_macro_signatures(&db, db.project());
        let inner = signatures
            .iter()
            .find(|signature| signature.definition.name == "inner")
            .unwrap();
        assert!(
            inner
                .parameters
                .iter()
                .any(|parameter| parameter.name == "delay"
                    && parameter.kinds.contains(&ValueKind::Number))
        );
        assert!(
            inner
                .parameters
                .iter()
                .any(|parameter| parameter.name == "count"
                    && parameter.js_types.intersects(JsTypeSet::NUMBER))
        );
        assert!(
            inner
                .parameters
                .iter()
                .any(|parameter| parameter.name == "enabled"
                    && parameter.js_types.intersects(JsTypeSet::BOOLEAN))
        );
        assert!(
            inner
                .parameters
                .iter()
                .any(|parameter| parameter.name == "title")
        );
        let outer = signatures
            .iter()
            .find(|signature| signature.definition.name == "outer")
            .unwrap();
        assert!(
            outer
                .parameters
                .iter()
                .any(|parameter| parameter.name == "time"
                    && parameter.kinds.contains(&ValueKind::Number))
        );
        assert!(
            outer
                .parameters
                .iter()
                .any(|parameter| parameter.name == "total"
                    && parameter.js_types.intersects(JsTypeSet::NUMBER))
        );
    }

    #[test]
    fn star_forwarding_exposes_builtin_arguments_on_the_macro() {
        let db = ProjectBuilder::new()
            .file(
                "data/scenario/first.ks",
                "[macro name=show_img][image *][endmacro]\n[show_img storage=ok.png layer=0]\n",
            )
            .build();
        let signatures = project_macro_signatures(&db, db.project());
        let signature = signatures
            .iter()
            .find(|signature| signature.definition.name == "show_img")
            .unwrap()
            .clone();

        let storage = signature
            .parameters
            .iter()
            .find(|parameter| parameter.name == "storage")
            .unwrap();
        assert!(storage.required);
        assert!(
            storage
                .kinds
                .contains(&ValueKind::Asset(tyrano_project::AssetKind::Image))
        );
        assert!(
            signature
                .parameters
                .iter()
                .any(|parameter| parameter.name == "layer")
        );
        assert!(signature.accepts_extra);
    }

    #[test]
    fn conditional_star_forwarding_keeps_the_selected_callee_shape() {
        let db = ProjectBuilder::new()
            .file(
                "data/scenario/first.ks",
                "[macro name=chara_mask]\n[if exp=\"mp.type === 'show'\"][chara_show *]\n[elsif exp=\"mp.type === 'mod'\"][chara_mod *]\n[endif]\n[endmacro]\n[chara_mask type=mod name=sui face=sad wait=false]\n",
            )
            .build();
        let signatures = project_macro_signatures(&db, db.project());
        let signature = signatures
            .iter()
            .find(|signature| signature.definition.name == "chara_mask")
            .unwrap();
        let mod_branch = signature
            .conditional
            .iter()
            .find(|branch| {
                branch.conditions
                    == [MacroCondition {
                        parameter: "type".to_owned(),
                        equals: "mod".to_owned(),
                    }]
            })
            .expect("the mod branch is conditional on mp.type");
        assert!(
            mod_branch
                .parameters
                .iter()
                .any(|parameter| parameter.name == "name")
        );
        assert!(
            mod_branch
                .parameters
                .iter()
                .any(|parameter| parameter.name == "face")
        );
        assert!(
            !mod_branch
                .parameters
                .iter()
                .any(|parameter| parameter.name == "wait"),
            "wait belongs to chara_show, not chara_mod"
        );
    }
}
