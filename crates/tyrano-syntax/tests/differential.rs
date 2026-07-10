//! Differential testing: the new lossless stack against the old
//! engine-compatible stack (`tyrano-lexer` + the runtime `LRParser`).
//!
//! The old pipeline destroys information while lexing (line trims, escape
//! consumption, quote stripping, quirk truncation); the new pipeline keeps
//! everything in the CST and applies the same interpretations lazily in
//! the AST view layer. This suite proves both pipelines *mean* the same
//! thing: converting the new typed AST view into the old `AstNode` shape
//! must reproduce the old parser's output exactly, for every quirk
//! configuration.
//!
//! The old parser has no error recovery, so inputs it rejects are only
//! checked for the new stack's invariants (tree produced, byte-exact
//! round-trip).

use tyrano_lexer::ParserConfig;
use tyrano_lexer::config::KeepSpaceLevel as OldKeep;
use tyrano_parser_generator::ast::{AstNode as OldAst, Parameter};
use tyrano_parser_generator::generator::TableGenerator;
use tyrano_parser_generator::grammar::GrammarParser;
use tyrano_parser_generator::parser::LRParser;
use tyrano_syntax::ast::{AstNode as _, InterpretOptions, KeepSpaceLevel, Line, TextSegment};
use tyrano_syntax::red::SyntaxNode;
use tyrano_syntax::{ParseOptions, SyntaxKind, parse_with_options};

const GRAMMAR: &str =
    include_str!(concat!(env!("CARGO_MANIFEST_DIR"), "/../../grammar/tyranoscript.grammar"));

// LRParser holds Rc-based grammar data (not Sync), so cache it per thread.
thread_local! {
    static OLD_PARSER: LRParser = {
        let grammar = GrammarParser::from_content(GRAMMAR).expect("grammar should load");
        let table = TableGenerator::new(grammar.clone())
            .expect("state machine should build")
            .generate_parse_table()
            .expect("parse table should build without conflicts");
        LRParser::new(table, grammar)
    };
}

fn old_options(config: &ParserConfig) -> (ParseOptions, InterpretOptions) {
    (
        ParseOptions { loose_endscript_termination: config.loose_endscript_termination },
        InterpretOptions {
            keep_space: match config.keep_space_in_parameter_value {
                OldKeep::RemoveAll => KeepSpaceLevel::RemoveAll,
                OldKeep::TrimEnds => KeepSpaceLevel::TrimEnds,
                OldKeep::KeepAll => KeepSpaceLevel::KeepAll,
            },
            label_value_first_segment_only: config.label_value_first_segment_only,
            chara_face_first_segment_only: config.chara_face_first_segment_only,
        },
    )
}

/// The engine's text-content escape rule: `\x` → `x`, trailing `\` dropped.
fn resolve_escapes(raw: &str) -> String {
    let mut out = String::with_capacity(raw.len());
    let mut chars = raw.chars();
    while let Some(c) = chars.next() {
        if c == '\\' {
            if let Some(next) = chars.next() {
                out.push(next);
            }
        } else {
            out.push(c);
        }
    }
    out
}

/// Converts the new typed AST view into the old `AstNode` shape.
fn new_as_old(source: &str, config: &ParserConfig) -> OldAst {
    let (po, io) = old_options(config);
    let parsed = parse_with_options(source, &po);
    assert_eq!(parsed.to_source(), source, "round-trip must hold");

    let mut lines: Vec<Box<OldAst>> = Vec::new();
    for line in parsed.ast().lines() {
        match &line {
            Line::Text(t) => {
                let preserve = t.preserves_whitespace();
                let segs = t.segments();
                for (i, seg) in segs.iter().enumerate() {
                    match seg {
                        TextSegment::Text(tok) => {
                            let mut s = resolve_escapes(tok.text());
                            // The engine trims the whole line, which for
                            // text content only affects the final segment.
                            if i == segs.len() - 1 {
                                s.truncate(s.trim_end().len());
                            }
                            if !s.is_empty() {
                                lines.push(Box::new(OldAst::Text {
                                    content: s,
                                    preserve_whitespace: preserve,
                                }));
                            }
                        }
                        TextSegment::Tag(tag) => lines.push(Box::new(conv_tag(tag, false, &io))),
                    }
                }
            }
            Line::Label(l) => lines.push(Box::new(OldAst::Label {
                name: l.name().unwrap_or_default(),
                text: l.value(&io),
            })),
            Line::Chara(c) => lines.push(Box::new(OldAst::CharacterName {
                name: c.name().unwrap_or_default(),
                face: c.face(&io),
            })),
            Line::Comment(c) => lines.push(Box::new(OldAst::Comment {
                content: format!(";{}", c.text().unwrap_or_default().trim_end()),
                is_block: false,
            })),
            Line::BlockComment(b) => {
                lines.push(Box::new(OldAst::Comment { content: "/*".into(), is_block: true }));
                let closed = b
                    .syntax()
                    .descendants_with_tokens()
                    .filter_map(|e| e.into_token())
                    .any(|t| t.kind() == SyntaxKind::STAR_SLASH && !t.is_missing());
                if closed {
                    lines
                        .push(Box::new(OldAst::Comment { content: "*/".into(), is_block: true }));
                }
            }
            Line::AtTag(t) => lines.push(Box::new(conv_tag(t, true, &io))),
            Line::IScript(s) => lines.push(Box::new(OldAst::Script {
                content: old_block_content(s.syntax(), SyntaxKind::SCRIPT_TEXT),
            })),
            Line::Html(h) => lines.push(Box::new(OldAst::Html {
                content: old_block_content(h.syntax(), SyntaxKind::HTML_TEXT),
            })),
            Line::Error(_) => {
                unreachable!("differential corpus must be old-parseable: {source:?}")
            }
        }
    }
    OldAst::Scenario { lines }
}

fn conv_tag(tag: &impl tyrano_syntax::ast::Tag, at: bool, io: &InterpretOptions) -> OldAst {
    OldAst::Tag {
        name: tag.name(),
        parameters: tag
            .params()
            .iter()
            .map(|p| Parameter { name: p.name(), value: p.cooked_value(io) })
            .collect(),
        is_at_notation: at,
    }
}

/// Rebuilds the old `script_content`/`html_content` concatenation: every
/// interior NEWLINE contributes `"\n"`, every raw line its trimmed text.
/// The opener's own newline is a direct child for inline openers but lives
/// inside the AT_TAG_LINE node for `@iscript` openers; closer lines
/// contribute nothing (their newline belonged to the outer line list).
fn old_block_content(block: &SyntaxNode, raw_kind: SyntaxKind) -> String {
    let mut out = String::new();
    let mut seen_node = false;
    for el in block.children_with_tokens() {
        match el {
            tyrano_syntax::red::SyntaxElement::Token(t) => {
                if t.kind() == SyntaxKind::NEWLINE {
                    out.push('\n');
                } else if t.kind() == raw_kind {
                    out.push_str(t.text().trim());
                }
            }
            tyrano_syntax::red::SyntaxElement::Node(n) => {
                if !seen_node && n.kind() == SyntaxKind::AT_TAG_LINE {
                    // `@iscript` opener: its newline was part of the old
                    // block content.
                    let has_newline = n
                        .descendants_with_tokens()
                        .filter_map(|e| e.into_token())
                        .any(|t| t.kind() == SyntaxKind::NEWLINE);
                    if has_newline {
                        out.push('\n');
                    }
                }
                seen_node = true;
            }
        }
    }
    out
}

/// Compares old-parser output with the converted new-parser output for
/// one (source, config) pair. Inputs the old parser rejects only assert
/// the new stack's invariants.
fn check(source: &str, config: &ParserConfig) {
    let old = OLD_PARSER.with(|p| p.parse_with_config(source, config.clone()));
    match old {
        Ok(old_ast) => {
            let new_ast = new_as_old(source, config);
            assert_eq!(
                format!("{old_ast:#?}"),
                format!("{new_ast:#?}"),
                "AST divergence for {source:?} with {config:?}"
            );
        }
        Err(_) => {
            // No recovery in the old parser; the new one must still
            // produce a lossless tree.
            let (po, _) = old_options(config);
            let parsed = parse_with_options(source, &po);
            assert_eq!(parsed.to_source(), source);
        }
    }
}

fn check_default(source: &str) {
    check(source, &ParserConfig::default());
}

#[test]
fn corpus_files_match() {
    let dir = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../debug_artifacts");
    let mut seen = 0;
    for entry in std::fs::read_dir(dir).expect("corpus dir") {
        let path = entry.expect("entry").path();
        if path.extension().is_some_and(|e| e == "ks") {
            check_default(&std::fs::read_to_string(&path).expect("readable"));
            seen += 1;
        }
    }
    assert!(seen >= 3, "expected the bundled corpus");
}

#[test]
fn tags_and_parameters_match() {
    for src in [
        "[bg storage=room.jpg time=1000]",
        "@jump storage=\"title.ks\"",
        "[macro_use * flag2]",
        "[a t=]",
        "[a t=\"undefined\"]",
        "[a t=undefined]",
        "[eval exp=&f.name cond=%flag]",
        "[eval exp=f.a[0]]",
        "[3d_init]",
        "[a p = 1]",
        "[ptext text=\"[[あ]]\"]",
        "[wait time=200][l][r]",
        "@bg storage = room.jpg\n",
    ] {
        check_default(src);
    }
}

#[test]
fn labels_and_charas_match() {
    for src in [
        "*start",
        "*start|セーブ1",
        "* start | 題名 ",
        "*a|b|c",
        "*|value",
        "#akane:happy",
        "#やまだ",
        "#",
        "#a:b:c",
        "#a: b",
        "#:face",
    ] {
        check_default(src);
        for label_flag in [true, false] {
            for chara_flag in [true, false] {
                let config = ParserConfig {
                    label_value_first_segment_only: label_flag,
                    chara_face_first_segment_only: chara_flag,
                    ..ParserConfig::default()
                };
                check(src, &config);
            }
        }
    }
}

#[test]
fn text_and_comments_match() {
    for src in [
        "こんにちは",
        "  こんにちは  ",
        "こんにちは[l]世界",
        "_  インデント保持",
        "\\[not a tag\\]",
        "a;b",
        ";コメント",
        ";",
        "/*\nhidden\n*/",
        "/*\nnever closed",
        "*/",
        "[l] [r]",
    ] {
        check_default(src);
    }
}

#[test]
fn keep_space_levels_match() {
    for level in [OldKeep::RemoveAll, OldKeep::TrimEnds, OldKeep::KeepAll] {
        let config = ParserConfig::default().with_keep_space(level);
        for src in [
            "[a t=\" x y \"]",
            "[a t=` x y `]",
            "[a t=' x y ']",
            "[a t=\" undefined \"]",
            "[a t=\"a\\ b\"]",
        ] {
            check(src, &config);
        }
    }
}

#[test]
fn blocks_match() {
    for src in [
        "[iscript]\nvar a = 1;\n[endscript]",
        "[iscript]\nvar a = 1;\n\n  indented\n[endscript]\n",
        "@iscript\ncode\n@endscript\n",
        "[html]\n<b>bold</b>\n[endhtml]\n",
        "[iscript]\nvar s = \"endscript\";\n[s]",
        "[iscript]\nunterminated",
        "[endscript2]",
    ] {
        check_default(src);
        check(
            src,
            &ParserConfig { loose_endscript_termination: false, ..ParserConfig::default() },
        );
    }
}

#[test]
fn mixed_scenario_matches() {
    check_default(
        "*start|オープニング\n\
         #akane:happy\n\
         こんにちは[l]世界[r]\n\
         @bg storage=room.jpg time=1000\n\
         ;コメント行\n\
         /*\n\
         メモ\n\
         */\n\
         [iscript]\n\
         var a = 1;\n\
         [endscript]\n\
         _  保持テキスト\n\
         [macro_use * flag2]\n\
         \n\
         *end\n",
    );
}
