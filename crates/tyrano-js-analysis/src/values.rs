use std::collections::HashMap;

use oxc_ast::{AstKind, ast::*};
use oxc_semantic::{Semantic, SymbolId};
use oxc_span::{GetSpan, Span};
use oxc_syntax::operator::{AssignmentOperator, BinaryOperator, UnaryOperator};

use crate::path::{contains, lexical_binding_path, path_from_expression};
use crate::{
    EffectKind, EffectRole, JsAssignment, JsEffect, JsPathConstraint, JsStaticEquality, JsTypeSet,
    JsValueExpr, TextRange, VariablePath,
};

pub(crate) struct ValueFacts {
    pub assignments: Vec<JsAssignment>,
    pub constraints: Vec<JsPathConstraint>,
    pub equalities: Vec<JsStaticEquality>,
}

pub(crate) fn lower_value_facts(
    semantic: &Semantic<'_>,
    aliases: &HashMap<SymbolId, VariablePath>,
    effects: &[JsEffect],
) -> ValueFacts {
    let mut assignments = Vec::new();
    let mut constraints = Vec::new();
    let mut equalities = Vec::new();
    for (_node_id, node) in semantic.nodes().iter_enumerated() {
        match node.kind() {
            AstKind::VariableDeclarator(declarator) => {
                let (Some(binding), Some(initializer)) = (
                    declarator.id.get_binding_identifier(),
                    declarator.init.as_ref(),
                ) else {
                    continue;
                };
                let target = lexical_binding_path(binding);
                let (deferred, conditional) = effects
                    .iter()
                    .find(|effect| {
                        effect.kind == EffectKind::Define
                            && effect.selection == TextRange::from(binding.span)
                    })
                    .map_or((false, false), |effect| {
                        (effect.deferred, effect.conditional)
                    });
                assignments.push(JsAssignment {
                    target,
                    value: lower_value(initializer, semantic, aliases),
                    range: declarator.span.into(),
                    deferred,
                    conditional,
                    order: declarator.span.end,
                });
            }
            AstKind::AssignmentExpression(assignment) => {
                let target_span = assignment.left.span();
                let Some(write) = effects.iter().find(|effect| {
                    effect.kind == EffectKind::Write
                        && effect.role == EffectRole::Value
                        && effect.order == assignment.span.end
                        && contains(
                            target_span,
                            Span::new(effect.selection.start, effect.selection.end),
                        )
                }) else {
                    continue;
                };
                let right = lower_value(&assignment.right, semantic, aliases);
                let value = match assignment.operator {
                    AssignmentOperator::Assign => right,
                    AssignmentOperator::Addition => JsValueExpr::Add(
                        Box::new(JsValueExpr::Path(write.path.clone())),
                        Box::new(right),
                    ),
                    operator if operator.is_arithmetic() || operator.is_bitwise() => {
                        JsValueExpr::Typed(JsTypeSet::NUMBER)
                    }
                    _ => JsValueExpr::Choice(vec![JsValueExpr::Path(write.path.clone()), right]),
                };
                assignments.push(JsAssignment {
                    target: write.path.clone(),
                    value,
                    range: assignment.span.into(),
                    deferred: write.deferred,
                    conditional: write.conditional,
                    order: assignment.span.end,
                });
            }
            AstKind::BinaryExpression(binary) => {
                lower_binary_constraints(binary, effects, &mut constraints);
                lower_static_equality(binary, semantic, aliases, &mut equalities);
            }
            AstKind::UnaryExpression(unary)
                if matches!(
                    unary.operator,
                    UnaryOperator::UnaryPlus
                        | UnaryOperator::UnaryNegation
                        | UnaryOperator::BitwiseNot
                ) =>
            {
                add_constraints_for_span(
                    unary.argument.span(),
                    JsTypeSet::NUMBER,
                    effects,
                    &mut constraints,
                );
            }
            _ => {}
        }
    }
    assignments.sort_by_key(|assignment| assignment.order);
    constraints.sort_by_key(|constraint| (constraint.range.start, constraint.range.end));
    constraints.dedup();
    equalities.sort_by_key(|equality| (equality.range.start, equality.range.end));
    equalities.dedup();
    ValueFacts {
        assignments,
        constraints,
        equalities,
    }
}

fn lower_static_equality(
    binary: &BinaryExpression<'_>,
    semantic: &Semantic<'_>,
    aliases: &HashMap<SymbolId, VariablePath>,
    out: &mut Vec<JsStaticEquality>,
) {
    if !binary.operator.is_equality() {
        return;
    }
    let equality = |path: &Expression<'_>, value: &Expression<'_>| {
        let Expression::StringLiteral(value) = value.get_inner_expression() else {
            return None;
        };
        Some(JsStaticEquality {
            path: path_from_expression(path.get_inner_expression(), semantic, aliases)?,
            value: value.value.to_string(),
            range: binary.span.into(),
        })
    };
    if let Some(equality) =
        equality(&binary.left, &binary.right).or_else(|| equality(&binary.right, &binary.left))
    {
        out.push(equality);
    }
}

pub(crate) fn lower_expression_result(
    program: &Program<'_>,
    semantic: &Semantic<'_>,
    aliases: &HashMap<SymbolId, VariablePath>,
) -> Option<JsValueExpr> {
    if program.body.len() != 1 {
        return None;
    }
    let Statement::ExpressionStatement(statement) = &program.body[0] else {
        return None;
    };
    Some(lower_value(&statement.expression, semantic, aliases))
}

fn lower_value(
    expression: &Expression<'_>,
    semantic: &Semantic<'_>,
    aliases: &HashMap<SymbolId, VariablePath>,
) -> JsValueExpr {
    let inner = expression.get_inner_expression();
    match inner {
        Expression::BooleanLiteral(value) => JsValueExpr::Boolean(value.value),
        Expression::NullLiteral(_) => JsValueExpr::Null,
        Expression::NumericLiteral(value) => JsValueExpr::Number(value.value.to_string()),
        Expression::StringLiteral(value) => JsValueExpr::String(value.value.to_string()),
        Expression::Identifier(identifier)
            if identifier.name == "undefined" && identifier.reference_id.get().is_none() =>
        {
            JsValueExpr::Undefined
        }
        Expression::Identifier(_)
        | Expression::ComputedMemberExpression(_)
        | Expression::StaticMemberExpression(_)
        | Expression::PrivateFieldExpression(_)
        | Expression::ChainExpression(_) => path_from_expression(inner, semantic, aliases)
            .map(JsValueExpr::Path)
            .unwrap_or(JsValueExpr::Unknown),
        Expression::BinaryExpression(binary) => match binary.operator {
            BinaryOperator::Addition => JsValueExpr::Add(
                Box::new(lower_value(&binary.left, semantic, aliases)),
                Box::new(lower_value(&binary.right, semantic, aliases)),
            ),
            operator
                if operator.is_equality() || operator.is_compare() || operator.is_relational() =>
            {
                JsValueExpr::Typed(JsTypeSet::BOOLEAN)
            }
            _ => JsValueExpr::Typed(JsTypeSet::NUMBER),
        },
        Expression::LogicalExpression(logical) => JsValueExpr::Choice(vec![
            lower_value(&logical.left, semantic, aliases),
            lower_value(&logical.right, semantic, aliases),
        ]),
        Expression::ConditionalExpression(conditional) => JsValueExpr::Choice(vec![
            lower_value(&conditional.consequent, semantic, aliases),
            lower_value(&conditional.alternate, semantic, aliases),
        ]),
        Expression::AssignmentExpression(assignment) => {
            lower_value(&assignment.right, semantic, aliases)
        }
        Expression::SequenceExpression(sequence) => sequence
            .expressions
            .last()
            .map_or(JsValueExpr::Undefined, |value| {
                lower_value(value, semantic, aliases)
            }),
        Expression::ArrayExpression(array) => JsValueExpr::Array(
            array
                .elements
                .iter()
                .map(|element| {
                    element
                        .as_expression()
                        .map_or(JsValueExpr::Unknown, |value| {
                            lower_value(value, semantic, aliases)
                        })
                })
                .collect(),
        ),
        Expression::ObjectExpression(object) => JsValueExpr::Object(
            object
                .properties
                .iter()
                .filter_map(|property| {
                    let ObjectPropertyKind::ObjectProperty(property) = property else {
                        return None;
                    };
                    Some((
                        property.key.static_name()?.into_owned(),
                        lower_value(&property.value, semantic, aliases),
                    ))
                })
                .collect(),
        ),
        Expression::TemplateLiteral(template) => {
            let mut parts = Vec::with_capacity(template.quasis.len() + template.expressions.len());
            for (index, quasi) in template.quasis.iter().enumerate() {
                parts.push(JsValueExpr::String(
                    quasi
                        .value
                        .cooked
                        .as_ref()
                        .unwrap_or(&quasi.value.raw)
                        .to_string(),
                ));
                if let Some(expression) = template.expressions.get(index) {
                    parts.push(lower_value(expression, semantic, aliases));
                }
            }
            concat_parts(parts)
        }
        Expression::UnaryExpression(unary) => match unary.operator {
            UnaryOperator::LogicalNot | UnaryOperator::Delete => {
                JsValueExpr::Typed(JsTypeSet::BOOLEAN)
            }
            UnaryOperator::Typeof => JsValueExpr::Typed(JsTypeSet::STRING),
            UnaryOperator::Void => JsValueExpr::Undefined,
            _ => JsValueExpr::Typed(JsTypeSet::NUMBER),
        },
        Expression::ArrowFunctionExpression(_) | Expression::FunctionExpression(_) => {
            JsValueExpr::Function
        }
        Expression::ClassExpression(_)
        | Expression::NewExpression(_)
        | Expression::RegExpLiteral(_) => JsValueExpr::Typed(JsTypeSet::OBJECT),
        _ => JsValueExpr::Unknown,
    }
}

fn concat_parts(mut parts: Vec<JsValueExpr>) -> JsValueExpr {
    if parts.is_empty() {
        return JsValueExpr::String(String::new());
    }
    let first = parts.remove(0);
    parts.into_iter().fold(first, |left, right| {
        JsValueExpr::Add(Box::new(left), Box::new(right))
    })
}

fn lower_binary_constraints(
    binary: &BinaryExpression<'_>,
    effects: &[JsEffect],
    out: &mut Vec<JsPathConstraint>,
) {
    if binary.operator.is_arithmetic() && binary.operator != BinaryOperator::Addition
        || binary.operator.is_bitwise()
    {
        add_constraints_for_span(binary.left.span(), JsTypeSet::NUMBER, effects, out);
        add_constraints_for_span(binary.right.span(), JsTypeSet::NUMBER, effects, out);
        return;
    }
    if binary.operator.is_equality() || binary.operator.is_compare() {
        if let Some(expected) = literal_type(&binary.left) {
            add_constraints_for_span(binary.right.span(), expected, effects, out);
        }
        if let Some(expected) = literal_type(&binary.right) {
            add_constraints_for_span(binary.left.span(), expected, effects, out);
        }
    }
}

fn literal_type(expression: &Expression<'_>) -> Option<JsTypeSet> {
    match expression.get_inner_expression() {
        Expression::BooleanLiteral(_) => Some(JsTypeSet::BOOLEAN),
        Expression::NullLiteral(_) => Some(JsTypeSet::NULL),
        Expression::NumericLiteral(_) => Some(JsTypeSet::NUMBER),
        Expression::StringLiteral(_) | Expression::TemplateLiteral(_) => Some(JsTypeSet::STRING),
        _ => None,
    }
}

fn add_constraints_for_span(
    span: Span,
    expected: JsTypeSet,
    effects: &[JsEffect],
    out: &mut Vec<JsPathConstraint>,
) {
    for effect in effects.iter().filter(|effect| {
        effect.kind == EffectKind::Read
            && effect.role == EffectRole::Value
            && contains(
                span,
                Span::new(effect.selection.start, effect.selection.end),
            )
    }) {
        out.push(JsPathConstraint {
            path: effect.path.clone(),
            expected,
            range: span.into(),
        });
    }
}
