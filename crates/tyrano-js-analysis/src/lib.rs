//! Owned JavaScript facts for TyranoScript embedded-code analysis.
//!
//! Oxc's arena-backed AST and semantic model never escape [`analyze`]. The
//! returned facts are owned, lifetime-free values suitable for incremental
//! query outputs in the project semantic layer.

use std::collections::{HashMap, HashSet};

use oxc_allocator::Allocator;
use oxc_ast::{AstKind, ast::*};
use oxc_parser::Parser;
use oxc_semantic::{AstNodes, Semantic, SemanticBuilder, SymbolId};
use oxc_span::{GetSpan, SourceType, Span};

/// JavaScript source accepted by the embedded analyzer.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum SourceKind {
    /// An `[iscript]` body or loaded `.js` file.
    Script,
    /// An `exp=`, `cond=`, or `&entity` snippet.
    Expression,
}

/// A byte range relative to the analyzed JavaScript region.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct TextRange {
    pub start: u32,
    pub end: u32,
}

impl From<Span> for TextRange {
    fn from(span: Span) -> Self {
        Self {
            start: span.start,
            end: span.end,
        }
    }
}

/// A TyranoScript-owned variable namespace.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum EngineNamespace {
    F,
    Sf,
    Tf,
    Mp,
}

/// The binding at the root of a JavaScript variable path.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum VariableRoot {
    /// A canonical TyranoScript namespace, after alias normalization.
    Engine(EngineNamespace),
    /// A JavaScript lexical binding, identified by its definition site.
    Lexical { name: String, definition: TextRange },
    /// An unresolved JavaScript global other than an engine namespace.
    Global(String),
}

/// One property-selection step.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum PathSegment {
    Static(String),
    /// A computed property whose precise value is not statically known.
    Dynamic(TextRange),
}

/// An identifier or property path to which an effect applies.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct VariablePath {
    pub root: VariableRoot,
    pub segments: Vec<PathSegment>,
}

/// JavaScript operation performed on a variable path.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum EffectKind {
    Define,
    Read,
    Write,
    Execute,
}

/// Additional semantics carried by an effect.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum EffectRole {
    /// The path's value is consumed by the JavaScript operation.
    Value,
    /// The path must be non-nullish so a later property can be accessed.
    Dereference,
    /// A Write produced by `delete`; the target becomes undefined.
    Delete,
}

/// One arena-independent JavaScript variable effect.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct JsEffect {
    pub kind: EffectKind,
    pub role: EffectRole,
    pub path: VariablePath,
    /// The complete JavaScript operation this effect was lowered from.
    pub range: TextRange,
    /// The identifier/property token representing `path` at the source site.
    /// This is kept separate from `range` so IDE navigation can distinguish
    /// `f`, `player`, and `hp` in `f.player.hp` without changing effect
    /// evaluation or diagnostic ranges.
    pub selection: TextRange,
    /// True for effects inside a function/arrow body. Such effects exist in
    /// the program but do not run merely because the containing file loads.
    pub deferred: bool,
    /// True when enclosing control flow may skip this effect.
    pub conditional: bool,
    /// Relative evaluation order. Writes/calls use the enclosing operation's
    /// end so RHS/argument effects precede them.
    pub order: u32,
}

/// One resolved JavaScript lexical-binding occurrence used by IDE features.
///
/// This stays separate from [`JsEffect`]: an immutable alias such as
/// `const state = sf.game` must normalize effects through `state`, while Go
/// Definition on the spelling `state` must still target its lexical binding.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct JsBindingOccurrence {
    pub name: String,
    pub definition: TextRange,
    pub selection: TextRange,
    pub declaration: bool,
}

/// Arena-independent facts obtained from one JavaScript source region.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct OwnedJsFacts {
    /// Whether Oxc accepted the complete region without a parse/semantic error.
    pub parsed: bool,
    /// Number of AST nodes visited while lowering the region.
    pub node_count: u32,
    /// Variable effects, including lexical definitions and normalized engine accesses.
    pub effects: Vec<JsEffect>,
    /// Resolved lexical declarations and references for source navigation.
    pub bindings: Vec<JsBindingOccurrence>,
    /// Parse/semantic diagnostic messages copied out of Oxc.
    pub diagnostics: Vec<String>,
}

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

/// Parses and binds one JavaScript region, returning only owned facts.
pub fn analyze(source: &str, _kind: SourceKind) -> OwnedJsFacts {
    let allocator = Allocator::default();
    let parsed = Parser::new(&allocator, source, SourceType::default()).parse();
    let mut diagnostics: Vec<String> = parsed.diagnostics.iter().map(ToString::to_string).collect();
    let semantic_return = SemanticBuilder::new_compiler()
        .with_build_nodes(true)
        .build(&parsed.program);
    diagnostics.extend(semantic_return.diagnostics.iter().map(ToString::to_string));
    let semantic = semantic_return.semantic;
    let aliases = immutable_path_aliases(&semantic);
    let mut effects = lower_effects(&semantic, &aliases);
    effects.sort_by_key(|effect| effect.order);
    let mut seen = HashSet::new();
    effects.retain(|effect| seen.insert(effect.clone()));
    let bindings = lower_binding_occurrences(&semantic);

    OwnedJsFacts {
        parsed: diagnostics.is_empty(),
        node_count: semantic.nodes().len() as u32,
        effects,
        bindings,
        diagnostics,
    }
}

fn lower_binding_occurrences(semantic: &Semantic<'_>) -> Vec<JsBindingOccurrence> {
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

fn lower_effects(
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
    node_id: oxc_semantic::NodeId,
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

fn immutable_path_aliases(semantic: &Semantic<'_>) -> HashMap<SymbolId, VariablePath> {
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

fn lexical_binding_path(identifier: &BindingIdentifier<'_>) -> VariablePath {
    VariablePath {
        root: VariableRoot::Lexical {
            name: identifier.name.to_string(),
            definition: identifier.span.into(),
        },
        segments: Vec::new(),
    }
}

fn path_from_identifier(
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

fn path_from_expression(
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

fn path_from_member_node(
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
enum MemberNode<'a> {
    Computed(&'a ComputedMemberExpression<'a>),
    Static(&'a StaticMemberExpression<'a>),
    Private(&'a PrivateFieldExpression<'a>),
}

impl<'a> MemberNode<'a> {
    fn object(self) -> &'a Expression<'a> {
        match self {
            Self::Computed(member) => &member.object,
            Self::Static(member) => &member.object,
            Self::Private(member) => &member.object,
        }
    }

    fn span(self) -> Span {
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
fn member_source_selections(member: MemberNode<'_>) -> Vec<Span> {
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

fn component_selection(
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

fn identifier_is_member_root(
    nodes: &AstNodes<'_>,
    node_id: oxc_semantic::NodeId,
    span: Span,
) -> bool {
    nodes.ancestors(node_id).next().is_some_and(|parent| {
        member_object_span(parent.kind()).is_some_and(|object| contains(object, span))
    })
}

fn member_is_object_of_parent(
    nodes: &AstNodes<'_>,
    node_id: oxc_semantic::NodeId,
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

fn contains(container: Span, nested: Span) -> bool {
    container.start <= nested.start && container.end >= nested.end
}

fn is_transparent(kind: AstKind<'_>) -> bool {
    matches!(
        kind,
        AstKind::ChainExpression(_) | AstKind::ParenthesizedExpression(_)
    )
}

fn is_deferred(nodes: &AstNodes<'_>, node_id: oxc_semantic::NodeId) -> bool {
    nodes.ancestors(node_id).any(|parent| {
        matches!(
            parent.kind(),
            AstKind::Function(_) | AstKind::ArrowFunctionExpression(_)
        )
    })
}

fn execution_flags(
    nodes: &AstNodes<'_>,
    node_id: oxc_semantic::NodeId,
    span: Span,
) -> ExecutionFlags {
    ExecutionFlags {
        deferred: is_deferred(nodes, node_id),
        conditional: is_conditional(nodes, node_id, span),
    }
}

fn is_conditional(nodes: &AstNodes<'_>, node_id: oxc_semantic::NodeId, span: Span) -> bool {
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

fn classify_access(nodes: &AstNodes<'_>, node_id: oxc_semantic::NodeId, span: Span) -> EffectKind {
    for parent in nodes.ancestors(node_id) {
        match parent.kind() {
            AstKind::ChainExpression(_) | AstKind::ParenthesizedExpression(_) => continue,
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

fn is_delete_access(nodes: &AstNodes<'_>, node_id: oxc_semantic::NodeId, span: Span) -> bool {
    for parent in nodes.ancestors(node_id) {
        match parent.kind() {
            AstKind::ChainExpression(_) | AstKind::ParenthesizedExpression(_) => continue,
            AstKind::UnaryExpression(unary) if contains(unary.argument.span(), span) => {
                return unary.operator.is_delete();
            }
            _ => return false,
        }
    }
    false
}

fn effect_order(
    nodes: &AstNodes<'_>,
    node_id: oxc_semantic::NodeId,
    span: Span,
    kind: EffectKind,
) -> u32 {
    if !matches!(kind, EffectKind::Write | EffectKind::Execute) {
        return span.start;
    }
    for parent in nodes.ancestors(node_id) {
        match parent.kind() {
            AstKind::ChainExpression(_) | AstKind::ParenthesizedExpression(_) => continue,
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

fn assignment_reads_old_value(
    nodes: &AstNodes<'_>,
    node_id: oxc_semantic::NodeId,
    span: Span,
) -> bool {
    for parent in nodes.ancestors(node_id) {
        match parent.kind() {
            AstKind::ChainExpression(_) | AstKind::ParenthesizedExpression(_) => continue,
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

#[cfg(test)]
mod tests {
    use super::*;

    fn assert_owned<T: 'static + Send + Sync + Clone + Eq>() {}

    fn engine_effect<'a>(
        facts: &'a OwnedJsFacts,
        namespace: EngineNamespace,
        properties: &[&str],
        kind: EffectKind,
        role: EffectRole,
    ) -> Option<&'a JsEffect> {
        facts.effects.iter().find(|effect| {
            effect.path.root == VariableRoot::Engine(namespace)
                && effect.path.segments
                    == properties
                        .iter()
                        .map(|name| PathSegment::Static((*name).to_owned()))
                        .collect::<Vec<_>>()
                && effect.kind == kind
                && effect.role == role
        })
    }

    #[test]
    fn facts_are_salsa_safe_owned_values() {
        assert_owned::<OwnedJsFacts>();
    }

    #[test]
    fn separates_lexical_definitions_and_normalizes_engine_aliases() {
        let facts = analyze(
            "let x = 1; f.player_name = 'Taro'; TYRANO.kag.stat.f.score = x;",
            SourceKind::Script,
        );
        assert!(facts.parsed, "{:?}", facts.diagnostics);
        assert!(facts.effects.iter().any(|effect| {
            effect.kind == EffectKind::Define
                && matches!(&effect.path.root, VariableRoot::Lexical { name, .. } if name == "x")
        }));
        assert!(
            engine_effect(
                &facts,
                EngineNamespace::F,
                &["player_name"],
                EffectKind::Write,
                EffectRole::Value,
            )
            .is_some()
        );
        assert!(
            engine_effect(
                &facts,
                EngineNamespace::F,
                &["score"],
                EffectKind::Write,
                EffectRole::Value,
            )
            .is_some()
        );
    }

    #[test]
    fn emits_intermediate_dereference_instead_of_defining_it() {
        let source = "f.player.hp = 100;";
        let facts = analyze(source, SourceKind::Script);
        let player = engine_effect(
            &facts,
            EngineNamespace::F,
            &["player"],
            EffectKind::Read,
            EffectRole::Dereference,
        )
        .unwrap();
        let hp = engine_effect(
            &facts,
            EngineNamespace::F,
            &["player", "hp"],
            EffectKind::Write,
            EffectRole::Value,
        )
        .unwrap();
        assert_eq!(
            &source[player.selection.start as usize..player.selection.end as usize],
            "player"
        );
        assert_eq!(
            &source[hp.selection.start as usize..hp.selection.end as usize],
            "hp"
        );
        assert!(!facts.effects.iter().any(|effect| {
            effect.kind == EffectKind::Define
                && effect.path.root == VariableRoot::Engine(EngineNamespace::F)
        }));
    }

    #[test]
    fn selection_survives_engine_prefix_normalization_and_alias_expansion() {
        let source = "const state = TYRANO.kag.variable.sf.game; state.score = 1;";
        let facts = analyze(source, SourceKind::Script);
        let score = engine_effect(
            &facts,
            EngineNamespace::Sf,
            &["game", "score"],
            EffectKind::Write,
            EffectRole::Value,
        )
        .unwrap();
        assert_eq!(
            &source[score.selection.start as usize..score.selection.end as usize],
            "score"
        );

        let game = facts
            .effects
            .iter()
            .find(|effect| {
                effect.role == EffectRole::Dereference
                    && effect.path.root == VariableRoot::Engine(EngineNamespace::Sf)
                    && effect.path.segments == [PathSegment::Static("game".to_owned())]
                    && effect.range.start > 40
            })
            .unwrap();
        assert_eq!(
            &source[game.selection.start as usize..game.selection.end as usize],
            "state"
        );
    }

    #[test]
    fn immutable_local_alias_keeps_engine_identity() {
        let facts = analyze(
            "const state = TYRANO.kag.variable.sf.game; state.score++;",
            SourceKind::Script,
        );
        assert!(facts.parsed, "{:?}", facts.diagnostics);
        assert!(
            engine_effect(
                &facts,
                EngineNamespace::Sf,
                &["game", "score"],
                EffectKind::Read,
                EffectRole::Value,
            )
            .is_some()
        );
        assert!(
            engine_effect(
                &facts,
                EngineNamespace::Sf,
                &["game", "score"],
                EffectKind::Write,
                EffectRole::Value,
            )
            .is_some()
        );
    }

    #[test]
    fn shadowed_engine_name_is_lexical() {
        let facts = analyze("function use(f) { f.value = 1; }", SourceKind::Script);
        assert!(
            !facts
                .effects
                .iter()
                .any(|effect| { effect.path.root == VariableRoot::Engine(EngineNamespace::F) })
        );
    }

    #[test]
    fn normalizes_iife_parameter_aliases_to_engine_paths() {
        let facts = analyze(
            "(function(TYRANO, mp) { mp.name; TYRANO.kag.stat.f.score = 1; })(window.TYRANO, window.TYRANO.kag.stat.mp);",
            SourceKind::Script,
        );
        assert!(facts.parsed, "{:?}", facts.diagnostics);
        assert!(
            engine_effect(
                &facts,
                EngineNamespace::Mp,
                &["name"],
                EffectKind::Read,
                EffectRole::Value,
            )
            .is_some()
        );
        assert!(
            engine_effect(
                &facts,
                EngineNamespace::F,
                &["score"],
                EffectKind::Write,
                EffectRole::Value,
            )
            .is_some()
        );
    }

    #[test]
    fn marks_called_path_as_execute() {
        let facts = analyze("sf.player.heal();", SourceKind::Script);
        assert!(
            engine_effect(
                &facts,
                EngineNamespace::Sf,
                &["player", "heal"],
                EffectKind::Execute,
                EffectRole::Value,
            )
            .is_some()
        );
    }

    #[test]
    fn classifies_delete_as_a_removing_write() {
        let facts = analyze("delete f.player.name;", SourceKind::Script);
        assert!(facts.parsed, "{:?}", facts.diagnostics);
        assert!(
            engine_effect(
                &facts,
                EngineNamespace::F,
                &["player", "name"],
                EffectKind::Write,
                EffectRole::Delete,
            )
            .is_some()
        );
    }

    #[test]
    fn marks_function_body_effects_as_deferred() {
        let facts = analyze(
            "function setup() { f.player = {}; } f.ready = true;",
            SourceKind::Script,
        );
        let player = engine_effect(
            &facts,
            EngineNamespace::F,
            &["player"],
            EffectKind::Write,
            EffectRole::Value,
        )
        .unwrap();
        let ready = engine_effect(
            &facts,
            EngineNamespace::F,
            &["ready"],
            EffectKind::Write,
            EffectRole::Value,
        )
        .unwrap();
        assert!(player.deferred);
        assert!(!ready.deferred);
    }

    #[test]
    fn marks_skippable_branch_effects_as_conditional() {
        let facts = analyze(
            "if (ready) f.player = {}; ready && (f.after = 1); f.root = 1;",
            SourceKind::Script,
        );
        let player = engine_effect(
            &facts,
            EngineNamespace::F,
            &["player"],
            EffectKind::Write,
            EffectRole::Value,
        )
        .unwrap();
        let after = engine_effect(
            &facts,
            EngineNamespace::F,
            &["after"],
            EffectKind::Write,
            EffectRole::Value,
        )
        .unwrap();
        let root = engine_effect(
            &facts,
            EngineNamespace::F,
            &["root"],
            EffectKind::Write,
            EffectRole::Value,
        )
        .unwrap();
        assert!(player.conditional);
        assert!(after.conditional);
        assert!(!root.conditional);
    }

    #[test]
    fn rhs_read_precedes_assignment_write() {
        let facts = analyze(
            "f.value = f.fallback; f.after = f.value;",
            SourceKind::Script,
        );
        let fallback = engine_effect(
            &facts,
            EngineNamespace::F,
            &["fallback"],
            EffectKind::Read,
            EffectRole::Value,
        )
        .unwrap();
        let write = engine_effect(
            &facts,
            EngineNamespace::F,
            &["value"],
            EffectKind::Write,
            EffectRole::Value,
        )
        .unwrap();
        let later_read = facts
            .effects
            .iter()
            .rfind(|effect| {
                effect.kind == EffectKind::Read
                    && effect.role == EffectRole::Value
                    && effect.path.root == VariableRoot::Engine(EngineNamespace::F)
                    && effect.path.segments == [PathSegment::Static("value".to_owned())]
            })
            .unwrap();
        assert!(fallback.order < write.order);
        assert!(write.order < later_read.order);
    }

    #[test]
    fn immutable_var_alias_preserves_generic_global_path() {
        let facts = analyze(
            "var tag = {}; tag.bg_rule = {}; var master_tag = TYRANO.kag.ftag.master_tag; for (var name in tag) { master_tag[name] = tag[name]; }",
            SourceKind::Script,
        );
        assert!(facts.effects.iter().any(|effect| {
            effect.kind == EffectKind::Write
                && matches!(&effect.path.root, VariableRoot::Lexical { name, .. } if name == "tag")
                && effect.path.segments == [PathSegment::Static("bg_rule".to_owned())]
        }));
        assert!(
            facts.effects.iter().any(|effect| {
                effect.kind == EffectKind::Write
                    && effect.path.root == VariableRoot::Global("TYRANO".to_owned())
                    && matches!(effect.path.segments.last(), Some(PathSegment::Dynamic(_)))
            }),
            "{:#?}",
            facts.effects
        );
    }
}
