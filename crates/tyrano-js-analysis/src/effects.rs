use std::collections::HashMap;

use oxc_ast::AstKind;
use oxc_semantic::{AstNodes, NodeId, Semantic, SymbolId};
use oxc_span::{GetSpan, Span};

use crate::path::{
    MemberNode, component_selection, contains, identifier_is_member_root, is_transparent,
    lexical_binding_path, member_is_object_of_parent, member_source_selections,
    path_from_identifier, path_from_member_node,
};
use crate::{EffectKind, EffectRole, JsEffect, VariablePath};

#[derive(Clone, Copy)]
struct ExecutionFlags {
    deferred: bool,
    conditional: bool,
}

#[derive(Clone, Copy)]
struct EffectSource {
    operation: Span,
    selection: Span,
}

impl EffectSource {
    fn identifier(span: Span) -> Self {
        Self {
            operation: span,
            selection: span,
        }
    }
}

pub(crate) fn lower_effects(
    semantic: &Semantic<'_>,
    aliases: &HashMap<SymbolId, VariablePath>,
) -> Vec<JsEffect> {
    let nodes = semantic.nodes();
    let mut effects = Vec::new();

    for (node_id, node) in nodes.iter_enumerated() {
        match node.kind() {
            AstKind::BindingIdentifier(identifier) => {
                let execution = execution_flags(nodes, node_id, identifier.span);
                effects.push(JsEffect {
                    kind: EffectKind::Define,
                    role: EffectRole::Value,
                    path: lexical_binding_path(identifier),
                    range: identifier.span.into(),
                    selection: identifier.span.into(),
                    deferred: execution.deferred,
                    conditional: execution.conditional,
                    order: identifier.span.start,
                });
            }
            AstKind::IdentifierReference(identifier) => {
                if identifier_is_member_root(nodes, node_id, identifier.span) {
                    continue;
                }
                let Some(path) = path_from_identifier(identifier, semantic, aliases) else {
                    continue;
                };
                let flags = identifier
                    .reference_id
                    .get()
                    .map(|id| semantic.scoping().get_reference(id).flags());
                let classified = classify_access(nodes, node_id, identifier.span);
                if classified == EffectKind::Execute {
                    push_terminal(
                        &mut effects,
                        path,
                        classified,
                        EffectSource::identifier(identifier.span),
                        effect_order(nodes, node_id, identifier.span, classified),
                        EffectRole::Value,
                        execution_flags(nodes, node_id, identifier.span),
                    );
                } else if classified == EffectKind::Write
                    && is_delete_access(nodes, node_id, identifier.span)
                {
                    push_terminal(
                        &mut effects,
                        path,
                        EffectKind::Write,
                        EffectSource::identifier(identifier.span),
                        identifier.span.start,
                        EffectRole::Delete,
                        execution_flags(nodes, node_id, identifier.span),
                    );
                } else if let Some(flags) = flags {
                    if flags.is_read() {
                        push_terminal(
                            &mut effects,
                            path.clone(),
                            EffectKind::Read,
                            EffectSource::identifier(identifier.span),
                            identifier.span.start,
                            EffectRole::Value,
                            execution_flags(nodes, node_id, identifier.span),
                        );
                    }
                    if flags.is_write() {
                        push_terminal(
                            &mut effects,
                            path,
                            EffectKind::Write,
                            EffectSource::identifier(identifier.span),
                            effect_order(nodes, node_id, identifier.span, EffectKind::Write),
                            EffectRole::Value,
                            execution_flags(nodes, node_id, identifier.span),
                        );
                    }
                }
            }
            AstKind::ComputedMemberExpression(member) => {
                lower_member_node(
                    semantic,
                    aliases,
                    nodes,
                    node_id,
                    MemberNode::Computed(member),
                    &mut effects,
                );
            }
            AstKind::StaticMemberExpression(member) => {
                lower_member_node(
                    semantic,
                    aliases,
                    nodes,
                    node_id,
                    MemberNode::Static(member),
                    &mut effects,
                );
            }
            AstKind::PrivateFieldExpression(member) => {
                lower_member_node(
                    semantic,
                    aliases,
                    nodes,
                    node_id,
                    MemberNode::Private(member),
                    &mut effects,
                );
            }
            _ => {}
        }
    }

    effects
}

fn lower_member_node(
    semantic: &Semantic<'_>,
    aliases: &HashMap<SymbolId, VariablePath>,
    nodes: &AstNodes<'_>,
    node_id: NodeId,
    member: MemberNode<'_>,
    effects: &mut Vec<JsEffect>,
) {
    let span = member.span();
    if member_is_object_of_parent(nodes, node_id, span) {
        return;
    }
    let Some(path) = path_from_member_node(member, semantic, aliases) else {
        return;
    };
    let selections = member_source_selections(member);
    let component_count = path.segments.len() + 1;

    for length in 0..path.segments.len() {
        let execution = execution_flags(nodes, node_id, span);
        effects.push(JsEffect {
            kind: EffectKind::Read,
            role: EffectRole::Dereference,
            path: VariablePath {
                root: path.root.clone(),
                segments: path.segments[..length].to_vec(),
            },
            range: span.into(),
            selection: component_selection(&selections, component_count, length).into(),
            deferred: execution.deferred,
            conditional: execution.conditional,
            order: span.start,
        });
    }
    let kind = classify_access(nodes, node_id, span);
    let role = if kind == EffectKind::Write && is_delete_access(nodes, node_id, span) {
        EffectRole::Delete
    } else {
        EffectRole::Value
    };
    push_terminal(
        effects,
        path.clone(),
        kind,
        EffectSource {
            operation: span,
            selection: component_selection(&selections, component_count, path.segments.len()),
        },
        effect_order(nodes, node_id, span, kind),
        role,
        execution_flags(nodes, node_id, span),
    );
    if kind == EffectKind::Write && assignment_reads_old_value(nodes, node_id, span) {
        push_terminal(
            effects,
            path,
            EffectKind::Read,
            EffectSource {
                operation: span,
                selection: component_selection(&selections, component_count, component_count - 1),
            },
            span.start,
            EffectRole::Value,
            execution_flags(nodes, node_id, span),
        );
    }
}

fn push_terminal(
    effects: &mut Vec<JsEffect>,
    path: VariablePath,
    kind: EffectKind,
    source: EffectSource,
    order: u32,
    role: EffectRole,
    execution: ExecutionFlags,
) {
    effects.push(JsEffect {
        kind,
        role,
        path,
        range: source.operation.into(),
        selection: source.selection.into(),
        deferred: execution.deferred,
        conditional: execution.conditional,
        order,
    });
}

fn is_deferred(nodes: &AstNodes<'_>, node_id: NodeId) -> bool {
    nodes.ancestors(node_id).any(|parent| {
        matches!(
            parent.kind(),
            AstKind::Function(_) | AstKind::ArrowFunctionExpression(_)
        )
    })
}

fn execution_flags(nodes: &AstNodes<'_>, node_id: NodeId, span: Span) -> ExecutionFlags {
    ExecutionFlags {
        deferred: is_deferred(nodes, node_id),
        conditional: is_conditional(nodes, node_id, span),
    }
}

fn is_conditional(nodes: &AstNodes<'_>, node_id: NodeId, span: Span) -> bool {
    nodes.ancestors(node_id).any(|parent| match parent.kind() {
        AstKind::IfStatement(statement) => !contains(statement.test.span(), span),
        AstKind::ConditionalExpression(expression) => !contains(expression.test.span(), span),
        AstKind::LogicalExpression(expression) => contains(expression.right.span(), span),
        AstKind::ForStatement(statement) => {
            contains(statement.body.span(), span)
                || statement
                    .update
                    .as_ref()
                    .is_some_and(|update| contains(update.span(), span))
        }
        AstKind::ForInStatement(statement) => !contains(statement.right.span(), span),
        AstKind::ForOfStatement(statement) => !contains(statement.right.span(), span),
        AstKind::WhileStatement(statement) => !contains(statement.test.span(), span),
        AstKind::SwitchStatement(statement) => !contains(statement.discriminant.span(), span),
        // Abrupt completion can skip any later operation in the try/catch;
        // treating the whole construct as optional is conservative.
        AstKind::TryStatement(_) | AstKind::CatchClause(_) => true,
        _ => false,
    })
}

fn classify_access(nodes: &AstNodes<'_>, node_id: NodeId, span: Span) -> EffectKind {
    for parent in nodes.ancestors(node_id) {
        match parent.kind() {
            kind if is_transparent(kind) => continue,
            AstKind::CallExpression(call) if contains(call.callee.span(), span) => {
                return EffectKind::Execute;
            }
            AstKind::NewExpression(call) if contains(call.callee.span(), span) => {
                return EffectKind::Execute;
            }
            AstKind::UnaryExpression(unary)
                if unary.operator.is_delete() && contains(unary.argument.span(), span) =>
            {
                return EffectKind::Write;
            }
            AstKind::AssignmentExpression(assignment) if contains(assignment.left.span(), span) => {
                return EffectKind::Write;
            }
            AstKind::UpdateExpression(update) if contains(update.argument.span(), span) => {
                return EffectKind::Write;
            }
            _ => return EffectKind::Read,
        }
    }
    EffectKind::Read
}

fn is_delete_access(nodes: &AstNodes<'_>, node_id: NodeId, span: Span) -> bool {
    for parent in nodes.ancestors(node_id) {
        match parent.kind() {
            kind if is_transparent(kind) => continue,
            AstKind::UnaryExpression(unary) if contains(unary.argument.span(), span) => {
                return unary.operator.is_delete();
            }
            _ => return false,
        }
    }
    false
}

fn effect_order(nodes: &AstNodes<'_>, node_id: NodeId, span: Span, kind: EffectKind) -> u32 {
    if !matches!(kind, EffectKind::Write | EffectKind::Execute) {
        return span.start;
    }
    for parent in nodes.ancestors(node_id) {
        match parent.kind() {
            parent_kind if is_transparent(parent_kind) => continue,
            AstKind::CallExpression(call)
                if kind == EffectKind::Execute && contains(call.callee.span(), span) =>
            {
                return call.span.end;
            }
            AstKind::NewExpression(call)
                if kind == EffectKind::Execute && contains(call.callee.span(), span) =>
            {
                return call.span.end;
            }
            AstKind::AssignmentExpression(assignment)
                if kind == EffectKind::Write && contains(assignment.left.span(), span) =>
            {
                return assignment.span.end;
            }
            AstKind::UpdateExpression(update)
                if kind == EffectKind::Write && contains(update.argument.span(), span) =>
            {
                return update.span.end;
            }
            _ => return span.start,
        }
    }
    span.start
}

fn assignment_reads_old_value(nodes: &AstNodes<'_>, node_id: NodeId, span: Span) -> bool {
    for parent in nodes.ancestors(node_id) {
        match parent.kind() {
            kind if is_transparent(kind) => continue,
            AstKind::AssignmentExpression(assignment) if contains(assignment.left.span(), span) => {
                return !assignment.operator.is_assign();
            }
            AstKind::UpdateExpression(update) if contains(update.argument.span(), span) => {
                return true;
            }
            _ => return false,
        }
    }
    false
}
