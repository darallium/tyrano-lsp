//! End-to-end parser compatibility tests: lexer + LALR tables + AST building
//! against real-world TyranoScript scenario snippets.

use tyrano_parser_generator::ast::AstNode;
use tyrano_lexer::ParserConfig;
use tyrano_parser_generator::generator::TableGenerator;
use tyrano_parser_generator::grammar::GrammarParser;
use tyrano_parser_generator::parser::LRParser;

const GRAMMAR: &str = include_str!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../grammar/tyranoscript.grammar"
));

// LRParser holds Rc-based grammar data (not Sync), so cache it per thread.
thread_local! {
    static PARSER: LRParser = {
        let grammar = GrammarParser::from_content(GRAMMAR).expect("grammar should load");
        let table = TableGenerator::new(grammar.clone())
            .expect("state machine should build")
            .generate_parse_table()
            .expect("parse table should build without conflicts");
        LRParser::new(table, grammar)
    };
}

fn parse(src: &str) -> AstNode {
    PARSER.with(|p| {
        p.parse(src)
            .unwrap_or_else(|e| panic!("parse failed for {src:?}: {e}"))
    })
}

fn parse_cfg(src: &str, config: ParserConfig) -> AstNode {
    PARSER.with(|p| {
        p.parse_with_config(src, config)
            .unwrap_or_else(|e| panic!("parse failed for {src:?}: {e}"))
    })
}

fn lines(ast: &AstNode) -> &[Box<AstNode>] {
    match ast {
        AstNode::Scenario { lines } => lines,
        other => panic!("expected Scenario at the top, got {other:?}"),
    }
}

/// Collect (name, value) pairs from a Tag node.
fn tag_params(node: &AstNode) -> Vec<(String, Option<String>)> {
    match node {
        AstNode::Tag { parameters, .. } => parameters
            .iter()
            .map(|p| (p.name.clone(), p.value.clone()))
            .collect(),
        other => panic!("expected Tag, got {other:?}"),
    }
}

fn tag_name(node: &AstNode) -> &str {
    match node {
        AstNode::Tag { name, .. } => name,
        other => panic!("expected Tag, got {other:?}"),
    }
}

// ---------------------------------------------------------------------------
// Whole-file smoke tests on the bundled samples
// ---------------------------------------------------------------------------

#[test]
fn parses_bundled_first_ks() {
    let src = include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../debug_artifacts/first.ks"
    ));
    parse(src);
}

#[test]
fn parses_bundled_title_ks() {
    let src = include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../debug_artifacts/title.ks"
    ));
    parse(src);
}

#[test]
fn parses_a_realistic_japanese_scenario() {
    let src = "\
*start|はじまり
;ここはコメント
#akane:happy
こんにちは、世界![l][r]
今日はいい天気ですね。[p]
@bg storage=room.jpg time=1000
[chara_show name=\"akane\" left=100 top=50]
_　全角スペースを保持したい行
[iscript]
var score = 100;
f.name = \"太郎\";
[endscript]
[s]
";
    parse(src);
}

// ---------------------------------------------------------------------------
// Tags
// ---------------------------------------------------------------------------

#[test]
fn bracket_tag_with_multiple_params() {
    let ast = parse("[bg storage=room.jpg time=1000]");
    let lines = lines(&ast);
    assert_eq!(lines.len(), 1, "expected one node, got {lines:?}");
    assert_eq!(tag_name(&lines[0]), "bg");
    assert_eq!(
        tag_params(&lines[0]),
        vec![
            ("storage".to_string(), Some("room.jpg".to_string())),
            ("time".to_string(), Some("1000".to_string())),
        ]
    );
    match &*lines[0] {
        AstNode::Tag { is_at_notation, .. } => assert!(!is_at_notation),
        _ => unreachable!(),
    }
}

#[test]
fn at_tag_sets_at_notation() {
    let ast = parse("@jump storage=\"title.ks\"");
    let lines = lines(&ast);
    assert_eq!(lines.len(), 1, "expected one node, got {lines:?}");
    match &*lines[0] {
        AstNode::Tag {
            name,
            parameters,
            is_at_notation,
        } => {
            assert_eq!(name, "jump");
            assert!(*is_at_notation);
            assert_eq!(parameters.len(), 1);
            assert_eq!(parameters[0].name, "storage");
            assert_eq!(parameters[0].value.as_deref(), Some("title.ks"));
        }
        other => panic!("expected Tag, got {other:?}"),
    }
}

#[test]
fn flag_param_has_no_value() {
    let ast = parse("[macro_use * flag2]");
    let lines = lines(&ast);
    assert_eq!(
        tag_params(&lines[0]),
        vec![
            ("*".to_string(), None),
            ("flag2".to_string(), None),
        ]
    );
}

#[test]
fn empty_value_param_is_some_empty() {
    let ast = parse("[a t=]");
    let lines = lines(&ast);
    assert_eq!(
        tag_params(&lines[0]),
        vec![("t".to_string(), Some(String::new()))]
    );
}

// ---------------------------------------------------------------------------
// Labels and character names
// ---------------------------------------------------------------------------

#[test]
fn label_ast() {
    let ast = parse("*start|セーブ1");
    match &*lines(&ast)[0] {
        AstNode::Label { name, text } => {
            assert_eq!(name, "start");
            assert_eq!(text.as_deref(), Some("セーブ1"));
        }
        other => panic!("expected Label, got {other:?}"),
    }
}

#[test]
fn label_without_title() {
    let ast = parse("*gamestart");
    match &*lines(&ast)[0] {
        AstNode::Label { name, text } => {
            assert_eq!(name, "gamestart");
            assert_eq!(*text, None);
        }
        other => panic!("expected Label, got {other:?}"),
    }
}

#[test]
fn label_extra_segments_preserved_with_config_off() {
    let cfg = ParserConfig {
        label_value_first_segment_only: false,
        ..ParserConfig::default()
    };
    let ast = parse_cfg("*a|b|c", cfg);
    match &*lines(&ast)[0] {
        AstNode::Label { name, text } => {
            assert_eq!(name, "a");
            assert_eq!(text.as_deref(), Some("b|c"));
        }
        other => panic!("expected Label, got {other:?}"),
    }
}

#[test]
fn character_name_ast() {
    let ast = parse("#akane:happy");
    match &*lines(&ast)[0] {
        AstNode::CharacterName { name, face } => {
            assert_eq!(name, "akane");
            assert_eq!(face.as_deref(), Some("happy"));
        }
        other => panic!("expected CharacterName, got {other:?}"),
    }
}

#[test]
fn sharp_alone_is_an_empty_character_name() {
    let ast = parse("#");
    match &*lines(&ast)[0] {
        AstNode::CharacterName { name, face } => {
            assert_eq!(name, "");
            assert_eq!(*face, None);
        }
        other => panic!("expected CharacterName, got {other:?}"),
    }
}

// ---------------------------------------------------------------------------
// Text
// ---------------------------------------------------------------------------

#[test]
fn japanese_text_with_inline_tag_ast() {
    let ast = parse("こんにちは[l]世界");
    let lines = lines(&ast);
    assert_eq!(lines.len(), 3, "expected text/tag/text, got {lines:?}");
    match &*lines[0] {
        AstNode::Text { content, .. } => assert_eq!(content, "こんにちは"),
        other => panic!("expected Text, got {other:?}"),
    }
    assert_eq!(tag_name(&lines[1]), "l");
    match &*lines[2] {
        AstNode::Text { content, .. } => assert_eq!(content, "世界"),
        other => panic!("expected Text, got {other:?}"),
    }
}

// ---------------------------------------------------------------------------
// iscript
// ---------------------------------------------------------------------------

#[test]
fn iscript_block_ast() {
    let ast = parse("[iscript]\nvar a = 1;\n[endscript]");
    let script = lines(&ast)
        .iter()
        .find_map(|n| match &**n {
            AstNode::Script { content } => Some(content.clone()),
            _ => None,
        })
        .expect("expected a Script node");
    assert!(script.contains("var a = 1;"), "script was {script:?}");
}

#[test]
fn loose_endscript_line_is_parsed_as_normal_content() {
    // 悪法 (default ON): a line merely containing "endscript" ends the block
    // and is itself parsed as an ordinary line.
    let ast = parse("[iscript]\nvar s = \"endscript\";\n[s]");
    let all = lines(&ast);
    assert!(
        all.iter().any(|n| matches!(&**n,
            AstNode::Text { content, .. } if content.contains("endscript"))),
        "expected the quirk line as Text, got {all:?}"
    );
    assert!(
        all.iter()
            .any(|n| matches!(&**n, AstNode::Tag { name, .. } if name == "s")),
        "expected the [s] tag, got {all:?}"
    );
}

// ---------------------------------------------------------------------------
// Comments
// ---------------------------------------------------------------------------

#[test]
fn line_comment_ast() {
    let ast = parse(";一番最初に呼び出されるファイル");
    match &*lines(&ast)[0] {
        AstNode::Comment { content, is_block } => {
            assert!(content.contains("一番最初"));
            assert!(!is_block);
        }
        other => panic!("expected Comment, got {other:?}"),
    }
}
