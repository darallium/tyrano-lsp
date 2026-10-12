use std::collections::HashMap;

use oxc_ast::{AstKind, ast::*};
use oxc_semantic::{AstNodes, NodeId, Semantic, SymbolId};
use oxc_span::{GetSpan, Span};

use crate::{EngineNamespace, PathSegment, VariablePath, VariableRoot};

pub(crate) fn immutable_path_aliases(semantic: &Semantic<'_>) -> HashMap<SymbolId, VariablePath> {
    let nodes = semantic.nodes();
    let mut aliases = HashMap::new();

    // Alias chains are normally source ordered, but a small fixed point also handles
    // declarations whose initializer refers to a later-bound immutable alias.
    loop {
        let before = aliases.len();
        for (node_id, node) in nodes.iter_enumerated() {
            match node.kind() {
                AstKind::VariableDeclarator(declarator)
                    if matches!(nodes.parent_kind(node_id), AstKind::VariableDeclaration(_)) =>
                {
                    let (Some(binding), Some(initializer)) = (
                        declarator.id.get_binding_identifier(),
                        declarator.init.as_ref(),
                    ) else {
                        continue;
                    };
                    insert_alias(binding, initializer, semantic, &mut aliases);
                }
                AstKind::CallExpression(call) => {
                    let parameters = match call.callee.get_inner_expression() {
                        Expression::FunctionExpression(function) => &function.params.items,
                        Expression::ArrowFunctionExpression(function) => &function.params.items,
                        _ => continue,
                    };
                    for (parameter, argument) in parameters.iter().zip(&call.arguments) {
                        let (Some(binding), Some(argument)) = (
                            parameter.pattern.get_binding_identifier(),
                            argument.as_expression(),
                        ) else {
                            continue;
                        };
                        insert_alias(binding, argument, semantic, &mut aliases);
                    }
                }
                _ => {}
            }
        }
        if aliases.len() == before {
            break;
        }
    }
    aliases
}

fn insert_alias(
    binding: &BindingIdentifier<'_>,
    value: &Expression<'_>,
    semantic: &Semantic<'_>,
    aliases: &mut HashMap<SymbolId, VariablePath>,
) {
    let Some(symbol_id) = binding.symbol_id.get() else {
        return;
    };
    if semantic.scoping().symbol_is_mutated(symbol_id) {
        return;
    }
    let Some(path) = path_from_expression(value, semantic, aliases) else {
        return;
    };
    aliases.entry(symbol_id).or_insert(path);
}

pub(crate) fn lexical_binding_path(identifier: &BindingIdentifier<'_>) -> VariablePath {
    VariablePath {
        root: VariableRoot::Lexical {
            name: identifier.name.to_string(),
            definition: identifier.span.into(),
        },
        segments: Vec::new(),
    }
}

pub(crate) fn path_from_identifier(
    identifier: &IdentifierReference<'_>,
    semantic: &Semantic<'_>,
    aliases: &HashMap<SymbolId, VariablePath>,
) -> Option<VariablePath> {
    let reference_id = identifier.reference_id.get()?;
    let reference = semantic.scoping().get_reference(reference_id);
    if let Some(symbol_id) = reference.symbol_id() {
        if let Some(path) = aliases.get(&symbol_id) {
            return Some(path.clone());
        }
        let definition = semantic.scoping().symbol_span(symbol_id).into();
        return Some(VariablePath {
            root: VariableRoot::Lexical {
                name: identifier.name.to_string(),
                definition,
            },
            segments: Vec::new(),
        });
    }

    let root = match identifier.name.as_str() {
        "f" => VariableRoot::Engine(EngineNamespace::F),
        "sf" => VariableRoot::Engine(EngineNamespace::Sf),
        "tf" => VariableRoot::Engine(EngineNamespace::Tf),
        "mp" => VariableRoot::Engine(EngineNamespace::Mp),
        name => VariableRoot::Global(name.to_owned()),
    };
    Some(VariablePath {
        root,
        segments: Vec::new(),
    })
}

pub(crate) fn path_from_expression(
    expression: &Expression<'_>,
    semantic: &Semantic<'_>,
    aliases: &HashMap<SymbolId, VariablePath>,
) -> Option<VariablePath> {
    match expression.get_inner_expression() {
        Expression::Identifier(identifier) => path_from_identifier(identifier, semantic, aliases),
        Expression::ChainExpression(chain) => {
            path_from_member_expression(chain.expression.as_member_expression()?, semantic, aliases)
        }
        expression => {
            path_from_member_expression(expression.as_member_expression()?, semantic, aliases)
        }
    }
}

fn path_from_member_expression(
    member: &MemberExpression<'_>,
    semantic: &Semantic<'_>,
    aliases: &HashMap<SymbolId, VariablePath>,
) -> Option<VariablePath> {
    let mut path = path_from_expression(member.object(), semantic, aliases)?;
    path.segments.push(member_segment(member));
    Some(normalize_engine_path(path))
}

pub(crate) fn path_from_member_node(
    member: MemberNode<'_>,
    semantic: &Semantic<'_>,
    aliases: &HashMap<SymbolId, VariablePath>,
) -> Option<VariablePath> {
    let mut path = path_from_expression(member.object(), semantic, aliases)?;
    path.segments.push(member.segment());
    Some(normalize_engine_path(path))
}

fn member_segment(member: &MemberExpression<'_>) -> PathSegment {
    member.static_property_name().map_or_else(
        || PathSegment::Dynamic(member.span().into()),
        |name| PathSegment::Static(name.to_owned()),
    )
}

fn normalize_engine_path(mut path: VariablePath) -> VariablePath {
    let VariableRoot::Global(root) = &path.root else {
        return path;
    };
    if matches!(root.as_str(), "window" | "globalThis")
        && matches!(path.segments.first(), Some(PathSegment::Static(name)) if name == "TYRANO")
    {
        path.root = VariableRoot::Global("TYRANO".to_owned());
        path.segments.remove(0);
    }
    let VariableRoot::Global(root) = &path.root else {
        return path;
    };
    if root != "TYRANO" || path.segments.len() < 3 {
        return path;
    }
    let static_name = |index: usize| match &path.segments[index] {
        PathSegment::Static(name) => Some(name.as_str()),
        PathSegment::Dynamic(_) => None,
    };
    if static_name(0) != Some("kag") {
        return path;
    }
    let namespace = match (static_name(1), static_name(2)) {
        (Some("variable" | "stat"), Some("f")) => EngineNamespace::F,
        (Some("variable"), Some("sf")) => EngineNamespace::Sf,
        (Some("variable"), Some("tf")) => EngineNamespace::Tf,
        (Some("variable" | "stat"), Some("mp")) => EngineNamespace::Mp,
        _ => return path,
    };
    path.root = VariableRoot::Engine(namespace);
    path.segments.drain(..3);
    path
}

#[derive(Clone, Copy)]
pub(crate) enum MemberNode<'a> {
    Computed(&'a ComputedMemberExpression<'a>),
    Static(&'a StaticMemberExpression<'a>),
    Private(&'a PrivateFieldExpression<'a>),
}

impl<'a> MemberNode<'a> {
    pub(crate) fn object(self) -> &'a Expression<'a> {
        match self {
            Self::Computed(member) => &member.object,
            Self::Static(member) => &member.object,
            Self::Private(member) => &member.object,
        }
    }

    pub(crate) fn span(self) -> Span {
        match self {
            Self::Computed(member) => member.span,
            Self::Static(member) => member.span,
            Self::Private(member) => member.span,
        }
    }

    fn segment(self) -> PathSegment {
        match self {
            Self::Computed(member) => member.static_property_name().map_or_else(
                || PathSegment::Dynamic(member.expression.span().into()),
                |name| PathSegment::Static(name.to_string()),
            ),
            Self::Static(member) => PathSegment::Static(member.property.name.to_string()),
            Self::Private(member) => PathSegment::Static(format!("#{}", member.field.name)),
        }
    }

    fn property_span(self) -> Span {
        match self {
            Self::Computed(member) => member.expression.span(),
            Self::Static(member) => member.property.span,
            Self::Private(member) => member.field.span,
        }
    }
}

/// Source components for a member chain, including its root. Canonical path
/// normalization can remove a prefix (`TYRANO.kag.stat.f`) or introduce one
/// through an immutable alias (`const state = sf.game`), so callers align the
/// source and canonical components from the terminal property backwards.
pub(crate) fn member_source_selections(member: MemberNode<'_>) -> Vec<Span> {
    let mut selections = expression_source_selections(member.object())
        .unwrap_or_else(|| vec![member.object().span()]);
    selections.push(member.property_span());
    selections
}

fn expression_source_selections(expression: &Expression<'_>) -> Option<Vec<Span>> {
    match expression.get_inner_expression() {
        Expression::Identifier(identifier) => Some(vec![identifier.span]),
        Expression::ChainExpression(chain) => {
            member_expression_source_selections(chain.expression.as_member_expression()?)
        }
        expression => member_expression_source_selections(expression.as_member_expression()?),
    }
}

fn member_expression_source_selections(member: &MemberExpression<'_>) -> Option<Vec<Span>> {
    let mut selections = expression_source_selections(member.object())?;
    let property = match member {
        MemberExpression::ComputedMemberExpression(member) => member.expression.span(),
        MemberExpression::StaticMemberExpression(member) => member.property.span,
        MemberExpression::PrivateFieldExpression(member) => member.field.span,
    };
    selections.push(property);
    Some(selections)
}

pub(crate) fn component_selection(
    selections: &[Span],
    canonical_component_count: usize,
    canonical_index: usize,
) -> Span {
    debug_assert!(!selections.is_empty());
    debug_assert!(canonical_index < canonical_component_count);
    let source_index =
        canonical_index as isize + selections.len() as isize - canonical_component_count as isize;
    let source_index = source_index.clamp(0, selections.len() as isize - 1) as usize;
    selections[source_index]
}

pub(crate) fn identifier_is_member_root(nodes: &AstNodes<'_>, node_id: NodeId, span: Span) -> bool {
    nodes.ancestors(node_id).next().is_some_and(|parent| {
        member_object_span(parent.kind()).is_some_and(|object| contains(object, span))
    })
}

pub(crate) fn member_is_object_of_parent(
    nodes: &AstNodes<'_>,
    node_id: NodeId,
    span: Span,
) -> bool {
    for parent in nodes.ancestors(node_id) {
        if let Some(object) = member_object_span(parent.kind()) {
            return contains(object, span);
        }
        if !is_transparent(parent.kind()) {
            return false;
        }
    }
    false
}

fn member_object_span(kind: AstKind<'_>) -> Option<Span> {
    match kind {
        AstKind::ComputedMemberExpression(member) => Some(member.object.span()),
        AstKind::StaticMemberExpression(member) => Some(member.object.span()),
        AstKind::PrivateFieldExpression(member) => Some(member.object.span()),
        _ => None,
    }
}

pub(crate) fn contains(container: Span, nested: Span) -> bool {
    container.start <= nested.start && container.end >= nested.end
}

pub(crate) fn is_transparent(kind: AstKind<'_>) -> bool {
    matches!(
        kind,
        AstKind::ChainExpression(_) | AstKind::ParenthesizedExpression(_)
    )
}
