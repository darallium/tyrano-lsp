use oxc_ast::AstKind;
use oxc_semantic::Semantic;

use crate::JsBindingOccurrence;

pub(crate) fn lower_binding_occurrences(semantic: &Semantic<'_>) -> Vec<JsBindingOccurrence> {
    let mut bindings = Vec::new();
    for node in semantic.nodes().iter() {
        match node.kind() {
            AstKind::BindingIdentifier(identifier) => {
                bindings.push(JsBindingOccurrence {
                    name: identifier.name.to_string(),
                    definition: identifier.span.into(),
                    selection: identifier.span.into(),
                    declaration: true,
                });
            }
            AstKind::IdentifierReference(identifier) => {
                let Some(reference_id) = identifier.reference_id.get() else {
                    continue;
                };
                let Some(symbol_id) = semantic.scoping().get_reference(reference_id).symbol_id()
                else {
                    continue;
                };
                bindings.push(JsBindingOccurrence {
                    name: identifier.name.to_string(),
                    definition: semantic.scoping().symbol_span(symbol_id).into(),
                    selection: identifier.span.into(),
                    declaration: false,
                });
            }
            _ => {}
        }
    }
    bindings.sort_by_key(|binding| (binding.selection.start, binding.selection.end));
    bindings.dedup();
    bindings
}
