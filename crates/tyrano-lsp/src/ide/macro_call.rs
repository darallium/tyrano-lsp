use std::collections::BTreeMap;

use tyrano_project::{File, ProjectDb};
use tyrano_semantic::{MacroCondition, MacroSignature, SelectedMacroSignature};
use tyrano_syntax::{ast::AnyTag, text::TextSize};

pub(super) fn arguments_at(
    db: &dyn ProjectDb,
    file: File,
    offset: TextSize,
) -> Option<BTreeMap<String, String>> {
    let module = tyrano_db::parsed_module(db, file.source(db));
    let node = module.syntax().find_node_at_offset(offset)?;
    let tag = node.ancestors().find_map(AnyTag::cast)?;
    Some(tyrano_semantic::static_macro_arguments(
        &tag.params(),
        &file.source(db).interpret_options(db),
    ))
}

pub(super) fn select(
    signature: &MacroSignature,
    arguments: &BTreeMap<String, String>,
) -> SelectedMacroSignature {
    tyrano_semantic::select_macro_signature(signature, arguments)
}

pub(super) fn conditions(conditions: &[MacroCondition]) -> String {
    conditions
        .iter()
        .map(|c| format!("{}={:?}", c.parameter, c.equals))
        .collect::<Vec<_>>()
        .join(" and ")
}

/// Preserve differing requirements/types under their own predicates.
pub(super) fn parameter_docs(
    selected: &SelectedMacroSignature,
    name: Option<&str>,
) -> Vec<(String, String)> {
    let mut out = Vec::new();
    let mut add = |parameters: &[tyrano_semantic::MacroParameterSpec],
                   condition: Option<String>| {
        for p in parameters
            .iter()
            .filter(|p| name.is_none_or(|n| n == p.name))
        {
            let requirement = if p.required { "required" } else { "optional" };
            let mut detail = format!("{}, {requirement}", super::hover::macro_parameter_types(p));
            if let Some(condition) = &condition {
                detail.push_str(&format!(" when {condition}"));
            }
            out.push((p.name.clone(), detail));
        }
    };
    add(&selected.parameters, None);
    for branch in &selected.pending {
        add(&branch.parameters, Some(conditions(&branch.conditions)));
    }
    out
}
