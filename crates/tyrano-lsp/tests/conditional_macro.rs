use tyrano_lsp::ide::{completions, hover};
use tyrano_project::{AssetKind, ProjectDatabase, ProjectDb, ProjectPath, testing::ProjectBuilder};
use tyrano_syntax::text::TextSize;

const MACRO: &str = r#"
[macro name=chara_mask]
[iscript]
mp = sf.chara_macro.checkValue(mp);
[endscript]
[if exp="mp !== false"]
[if exp="mp.type === 'move'"][chara_move *]
[elsif exp="mp.type === 'show'"][chara_show *]
[elsif exp="mp.type === 'mod'"][chara_mod *]
[elsif exp="mp.type === 'hide'"][chara_hide *]
[elsif exp="mp.type === 'hide_all'"][chara_hide_all *]
[elsif exp="mp.type === 'new'"][chara_new *]
[elsif exp="mp.type === 'face'"][chara_face *]
[endif]
[endif]
[endmacro]
"#;

fn fixture(call: &str) -> (ProjectDatabase, tyrano_project::File, TextSize) {
    let prefix = "[call storage=macro.ks]\n";
    let offset = prefix.len() + call.find('|').unwrap();
    let source = format!("{prefix}{}", call.replace('|', ""));
    let db = ProjectBuilder::new()
        .file("data/scenario/macro.ks", MACRO)
        .file("data/scenario/first.ks", &source)
        .asset(AssetKind::FgImage, "ao.png")
        .build();
    let file = db
        .file(&ProjectPath::new("data/scenario/first.ks").unwrap())
        .unwrap();
    (db, file, TextSize::new(offset as u32))
}

#[test]
fn branch_hover_and_parameter_requirements() {
    for (call, required, absent) in [
        (
            "[cha|ra_mask type=new]",
            "`storage` — fgimage asset, required",
            "`face`",
        ),
        (
            "[cha|ra_mask type=face]",
            "`face` — text, required",
            "`time`",
        ),
        (
            "[cha|ra_mask type=show]",
            "`face` — text, optional",
            "`jname`",
        ),
        (
            "[cha|ra_mask type=mod]",
            "`face` — text, optional",
            "`anim`",
        ),
        (
            "[cha|ra_mask type=move]",
            "`anim` — boolean, optional",
            "`face`",
        ),
        (
            "[cha|ra_mask type=hide]",
            "`name` — text, required",
            "`face`",
        ),
        (
            "[cha|ra_mask type=hide_all]",
            "`time` — number, optional",
            "`name`",
        ),
    ] {
        let (db, file, offset) = fixture(call);
        let doc = hover(&db, file, offset).unwrap().markdown;
        assert!(doc.contains(required), "{doc}");
        assert!(!doc.contains(absent), "{doc}");
    }
    let (db, file, offset) = fixture("[chara_mask na|me=ao type='new']");
    assert!(
        hover(&db, file, offset)
            .unwrap()
            .markdown
            .contains("text, required")
    );
}

#[test]
fn completion_uses_arguments_before_and_after_cursor() {
    for call in [
        "[chara_mask type=new |]",
        "[chara_mask | type=\"new\"]",
        "[chara_mask type='new' |",
        "@chara_mask type=new |",
        "台詞[chara_mask type=new |]",
    ] {
        let (db, file, offset) = fixture(call);
        let items = completions(&db, file, offset);
        let storage = items.iter().find(|i| i.label == "storage").expect(call);
        assert!(
            storage
                .detail
                .as_ref()
                .unwrap()
                .contains("fgimage asset, required")
        );
        assert!(
            !items.iter().any(|i| i.label == "face"),
            "{call}: {items:?}"
        );
    }
}

#[test]
fn unresolved_and_invalid_discriminants() {
    for discriminator in ["", "type=&f.kind ", "type=%kind "] {
        let (db, file, offset) = fixture(&format!("[chara_mask {discriminator}|]"));
        let items = completions(&db, file, offset);
        let faces = items
            .iter()
            .filter(|i| i.label == "face")
            .collect::<Vec<_>>();
        assert_eq!(faces.len(), 1);
        let detail = faces[0].detail.as_ref().unwrap();
        assert!(detail.contains("required when type=\"face\""), "{detail}");
        assert!(detail.contains("optional when type=\"show\""), "{detail}");
        assert!(
            !tyrano_semantic::check_file(&db, file)
                .iter()
                .any(|d| d.code == tyrano_semantic::codes::MISSING_PARAM)
        );
    }
    let (db, file, offset) = fixture("[cha|ra_mask]");
    let doc = hover(&db, file, offset).unwrap().markdown;
    assert!(doc.contains("when type=\"new\""));
    let (db, file, offset) = fixture("[chara_mask type=invalid |]");
    assert!(
        !completions(&db, file, offset)
            .iter()
            .any(|i| i.label == "name")
    );
}

#[test]
fn value_completion_and_asset_hover() {
    for (call, expected) in [
        ("[chara_mask type=move anim=|]", vec!["false", "true"]),
        ("[chara_mask type=hide_all page=|]", vec!["back", "fore"]),
        ("[chara_mask type=new storage=|]", vec!["ao.png"]),
        (
            "[chara_mask type=|]",
            vec!["face", "hide", "hide_all", "mod", "move", "new", "show"],
        ),
    ] {
        let (db, file, offset) = fixture(call);
        let items = completions(&db, file, offset);
        assert_eq!(
            items.iter().map(|i| i.label.as_str()).collect::<Vec<_>>(),
            expected
        );
    }
    let (db, file, offset) = fixture("[chara_mask type=new storage=a|o.png]");
    assert!(
        hover(&db, file, offset)
            .unwrap()
            .markdown
            .contains("data/fgimage/ao.png")
    );
}

#[test]
fn diagnostics_keep_selected_branch_only() {
    for (call, missing, mismatch) in [
        ("[chara_mask type=new]|", 2, 0),
        ("[chara_mask type=hide_all]|", 0, 0),
        ("[chara_mask type=move name=ao time=bad]|", 0, 1),
    ] {
        let (db, file, _) = fixture(call);
        let diagnostics = tyrano_semantic::check_file(&db, file);
        assert_eq!(
            diagnostics
                .iter()
                .filter(|d| d.code == tyrano_semantic::codes::MISSING_PARAM)
                .count(),
            missing
        );
        assert_eq!(
            diagnostics
                .iter()
                .filter(|d| d.code == tyrano_semantic::codes::KIND_MISMATCH)
                .count(),
            mismatch
        );
    }
}

#[test]
fn selection_preserves_multiple_predicates() {
    let db = ProjectBuilder::new()
        .file(
            "data/scenario/first.ks",
            r#"
[macro name=example]
[if exp="mp.kind === 'a'"]
[if exp="mp.mode === 'fast'"][wait *][endif]
[elsif exp="mp.kind === 'b'"][chara_show *][endif]
[endmacro]
"#,
        )
        .build();
    let signatures = tyrano_semantic::project_macro_signatures(&db, db.project());
    let signature = &signatures[0];
    let pending =
        tyrano_semantic::select_macro_signature(signature, &[("kind".into(), "a".into())].into());
    assert_eq!(pending.pending.len(), 1);
    assert_eq!(pending.pending[0].conditions.len(), 2);
    let selected = tyrano_semantic::select_macro_signature(
        signature,
        &[("kind".into(), "a".into()), ("mode".into(), "fast".into())].into(),
    );
    assert!(selected.pending.is_empty());
    assert_eq!(selected.matched.len(), 1);
}

#[test]
fn local_macro_keeps_differing_types_conditional() {
    let source = r#"
[macro name=example]
[if exp="mp.kind === 'image'"][image *]
[elsif exp="mp.kind === 'call'"][call *][endif]
[endmacro]
[example storage=ao.png]
"#;
    let db = ProjectBuilder::new()
        .file("data/scenario/first.ks", source)
        .build();
    let file = db
        .file(&ProjectPath::new("data/scenario/first.ks").unwrap())
        .unwrap();
    let offset = TextSize::new((source.rfind("storage=").unwrap() + 2) as u32);
    let doc = hover(&db, file, offset).unwrap().markdown;
    assert!(
        doc.contains("image asset, required when kind=\"image\""),
        "{doc}"
    );
    assert!(
        doc.contains("scenario file, optional when kind=\"call\""),
        "{doc}"
    );
    let offset = TextSize::new((source.rfind("[example").unwrap() + 2) as u32);
    let doc = hover(&db, file, offset).unwrap().markdown;
    assert!(doc.contains("kind=\"image\""));
    assert!(doc.contains("kind=\"call\""));
}

#[test]
fn unfinished_discriminant_does_not_select_branch() {
    let (db, file, offset) = fixture("[cha|ra_mask type=\"new");
    let doc = hover(&db, file, offset).unwrap().markdown;
    assert!(!doc.contains("Selected:"), "{doc}");
    assert!(doc.contains("when type=\"face\""), "{doc}");
}
