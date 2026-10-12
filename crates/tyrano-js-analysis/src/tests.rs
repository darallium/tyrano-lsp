use crate::*;

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

#[test]
fn keeps_structured_assignment_values_and_expression_result() {
    let script = analyze(
        "tf.base = 'config/'; tf.image = tf.base + 'button.png';",
        SourceKind::Script,
    );
    let image = script
        .assignments
        .iter()
        .find(|assignment| {
            assignment.target.root == VariableRoot::Engine(EngineNamespace::Tf)
                && assignment.target.segments == [PathSegment::Static("image".into())]
        })
        .expect("tf.image assignment");
    assert!(matches!(image.value, JsValueExpr::Add(_, _)));

    let expression = analyze("tf.base + 'button.png'", SourceKind::Expression);
    assert!(expression.parsed, "{:?}", expression.diagnostics);
    assert!(matches!(expression.result, Some(JsValueExpr::Add(_, _))));
    assert!(
        expression
            .result
            .as_ref()
            .unwrap()
            .possible_types()
            .intersects(JsTypeSet::STRING)
    );
}

#[test]
fn preserves_object_shapes_and_operation_type_constraints() {
    let facts = analyze(
        "var tag = { pm: { storage: 'default.png', time: 100 }, vital: ['storage'] }; mp.time * 2;",
        SourceKind::Script,
    );
    assert!(facts.assignments.iter().any(|assignment| {
        matches!(&assignment.value, JsValueExpr::Object(properties)
            if properties.iter().any(|(name, _)| name == "pm"))
    }));
    assert!(facts.constraints.iter().any(|constraint| {
        constraint.path.root == VariableRoot::Engine(EngineNamespace::Mp)
            && constraint.path.segments == [PathSegment::Static("time".into())]
            && constraint.expected == JsTypeSet::NUMBER
    }));
}
