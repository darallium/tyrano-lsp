//! Owned JavaScript facts for TyranoScript embedded-code analysis.
//!
//! Oxc's arena-backed AST and semantic model never escape [`analyze`]. The
//! returned facts are owned, lifetime-free values suitable for incremental
//! query outputs in the project semantic layer.

use std::collections::HashSet;

use oxc_allocator::Allocator;
use oxc_parser::Parser;
use oxc_semantic::SemanticBuilder;
use oxc_span::SourceType;

mod bindings;
mod effects;
mod model;
mod path;
mod values;

pub use model::{
    EffectKind, EffectRole, EngineNamespace, JsAssignment, JsBindingOccurrence, JsEffect,
    JsPathConstraint, JsStaticEquality, JsTypeSet, JsValueExpr, JsValueType, OwnedJsFacts,
    PathSegment, SourceKind, TextRange, VariablePath, VariableRoot,
};

use bindings::lower_binding_occurrences;
use effects::lower_effects;
use path::immutable_path_aliases;
use values::{lower_expression_result, lower_value_facts};

/// Parses and binds one JavaScript region, returning only owned facts.
pub fn analyze(source: &str, kind: SourceKind) -> OwnedJsFacts {
    let allocator = Allocator::default();
    let parsed = Parser::new(&allocator, source, SourceType::default()).parse();
    let mut diagnostics: Vec<String> = match kind {
        SourceKind::Script => parsed.diagnostics.iter().map(ToString::to_string).collect(),
        SourceKind::Expression => Parser::new(&allocator, source, SourceType::default())
            .parse_expression()
            .err()
            .into_iter()
            .flatten()
            .map(|diagnostic| diagnostic.to_string())
            .collect(),
    };
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
    let value_facts = lower_value_facts(&semantic, &aliases, &effects);
    let result = matches!(kind, SourceKind::Expression)
        .then(|| lower_expression_result(&parsed.program, &semantic, &aliases))
        .flatten();

    OwnedJsFacts {
        parsed: diagnostics.is_empty(),
        node_count: semantic.nodes().len() as u32,
        effects,
        bindings,
        assignments: value_facts.assignments,
        result,
        constraints: value_facts.constraints,
        equalities: value_facts.equalities,
        diagnostics,
    }
}

#[cfg(test)]
mod tests;
