//! Lexer compatibility tests against TyranoScript's reference implementation
//! (tyrano/plugins/kag/kag.parser.js: parseScenario + makeTag).
//!
//! Each test encodes observable behaviour of the official engine so that the
//! Rust lexer stays token-for-token compatible with real .ks scenarios.

use tyrano_lexer::config::{KeepSpaceLevel, ParserConfig};
use tyrano_lexer::{Scanner, TokenType};

fn lex_cfg(src: &str, config: ParserConfig) -> Vec<TokenType> {
    let mut scanner = Scanner::with_config(src, config);
    let tokens = scanner.scan_tokens().expect("lexing should succeed");
    tokens
        .into_iter()
        .map(|t| t.token_type)
        .filter(|t| *t != TokenType::Eof)
        .collect()
}

fn lex(src: &str) -> Vec<TokenType> {
    lex_cfg(src, ParserConfig::default())
}

fn text(s: &str) -> TokenType {
    TokenType::Text(s.to_string())
}
fn ident(s: &str) -> TokenType {
    TokenType::Identifier(s.to_string())
}
fn num(s: &str) -> TokenType {
    TokenType::Number(s.to_string())
}
fn string(s: &str) -> TokenType {
    TokenType::String(s.to_string())
}
fn comment(s: &str) -> TokenType {
    TokenType::LineComment(s.to_string())
}
fn script(s: &str) -> TokenType {
    TokenType::ScriptText(s.to_string())
}
fn html(s: &str) -> TokenType {
    TokenType::HtmlText(s.to_string())
}

use TokenType::{
    Asterisk, At, BlockCommentEnd, BlockCommentStart, Colon, Equal, HtmlEnd, HtmlStart,
    IscriptEnd, IscriptStart, LBracket, Newline, Pipe, RBracket, Sharp, Underscore,
};

// ---------------------------------------------------------------------------
// Text lines
// ---------------------------------------------------------------------------

#[test]
fn japanese_text_is_a_single_text_token() {
    assert_eq!(lex("こんにちは、世界!"), vec![text("こんにちは、世界!")]);
}

#[test]
fn ascii_text_line_is_text_not_identifier() {
    assert_eq!(lex("hello world"), vec![text("hello world")]);
}

#[test]
fn japanese_text_with_inline_tags() {
    assert_eq!(
        lex("こんにちは[l][r]"),
        vec![
            text("こんにちは"),
            LBracket,
            ident("l"),
            RBracket,
            LBracket,
            ident("r"),
            RBracket,
        ]
    );
}

#[test]
fn text_lines_are_trimmed_like_tyrano() {
    // kag.parser.js runs $.trim() on every line before dispatching.
    assert_eq!(lex("  こんにちは  "), vec![text("こんにちは")]);
}

#[test]
fn underscore_preserves_leading_whitespace_after_trim() {
    // Tyrano trims the line first, then strips a leading "_" so the
    // whitespace *after* the underscore survives.
    assert_eq!(
        lex("  _  こんにちは"),
        vec![Underscore, text("  こんにちは")]
    );
}

#[test]
fn underscore_alone_is_just_the_marker() {
    assert_eq!(lex("_"), vec![Underscore]);
}

#[test]
fn escaped_brackets_become_plain_text() {
    assert_eq!(lex(r"文字\[かっこ\]です"), vec![text("文字[かっこ]です")]);
}

#[test]
fn escaped_backslash_keeps_one_backslash() {
    assert_eq!(lex(r"a\\b"), vec![text(r"a\b")]);
}

#[test]
fn whitespace_between_inline_tags_is_kept_as_text() {
    // parseScenario pushes the " " between tags as a text object.
    assert_eq!(
        lex("[a] [b]"),
        vec![
            LBracket,
            ident("a"),
            RBracket,
            text(" "),
            LBracket,
            ident("b"),
            RBracket,
        ]
    );
}

// ---------------------------------------------------------------------------
// Comments
// ---------------------------------------------------------------------------

#[test]
fn semicolon_comment_at_line_start() {
    assert_eq!(lex(";コメント"), vec![comment(";コメント")]);
}

#[test]
fn semicolon_comment_after_leading_whitespace() {
    // The line is trimmed before the first-char check.
    assert_eq!(lex("  ;コメント"), vec![comment(";コメント")]);
}

#[test]
fn semicolon_in_the_middle_of_text_is_not_a_comment() {
    assert_eq!(lex("値段は100;円です"), vec![text("値段は100;円です")]);
}

#[test]
fn block_comment_hides_interior_lines() {
    assert_eq!(
        lex("/*\n[bg storage=a.jpg]\nこんにちは\n*/\nテキスト"),
        vec![
            BlockCommentStart,
            Newline,
            Newline,
            Newline,
            BlockCommentEnd,
            Newline,
            text("テキスト"),
        ]
    );
}

#[test]
fn block_comment_end_must_be_alone_on_its_line() {
    // "hoge */" does NOT close the comment in Tyrano.
    assert_eq!(
        lex("/*\nhoge */\n*/\nafter"),
        vec![
            BlockCommentStart,
            Newline,
            Newline,
            BlockCommentEnd,
            Newline,
            text("after"),
        ]
    );
}

#[test]
fn orphan_block_comment_end_is_a_label_named_slash() {
    // Outside a block comment, "*/" starts with '*' so Tyrano treats it as
    // a label whose name is "/".
    assert_eq!(lex("*/"), vec![Asterisk, text("/")]);
}

// ---------------------------------------------------------------------------
// Specials only at line start
// ---------------------------------------------------------------------------

#[test]
fn special_chars_mid_line_are_plain_text() {
    assert_eq!(lex("3 * 4 # x @ y"), vec![text("3 * 4 # x @ y")]);
}

// ---------------------------------------------------------------------------
// Labels
// ---------------------------------------------------------------------------

#[test]
fn simple_label() {
    assert_eq!(lex("*start"), vec![Asterisk, text("start")]);
}

#[test]
fn label_with_save_title() {
    assert_eq!(
        lex("*oped|オープニング"),
        vec![Asterisk, text("oped"), Pipe, text("オープニング")]
    );
}

#[test]
fn label_segments_are_trimmed() {
    assert_eq!(
        lex("* start | 題名 "),
        vec![Asterisk, text("start"), Pipe, text("題名")]
    );
}

#[test]
fn label_extra_pipe_segments_are_dropped_by_default() {
    // Tyrano: label_val = split("|")[1]; anything after a second "|" is lost.
    assert_eq!(
        lex("*a|b|c"),
        vec![Asterisk, text("a"), Pipe, text("b")]
    );
}

#[test]
fn label_extra_pipe_segments_can_be_preserved() {
    let cfg = ParserConfig {
        label_value_first_segment_only: false,
        ..ParserConfig::default()
    };
    assert_eq!(
        lex_cfg("*a|b|c", cfg),
        vec![Asterisk, text("a"), Pipe, text("b|c")]
    );
}

// ---------------------------------------------------------------------------
// Character names
// ---------------------------------------------------------------------------

#[test]
fn chara_name_with_face() {
    assert_eq!(
        lex("#akane:happy"),
        vec![Sharp, text("akane"), Colon, text("happy")]
    );
}

#[test]
fn chara_name_japanese() {
    assert_eq!(lex("#やまだ"), vec![Sharp, text("やまだ")]);
}

#[test]
fn sharp_alone_clears_the_name() {
    assert_eq!(lex("#"), vec![Sharp]);
}

#[test]
fn chara_face_segments_beyond_the_first_are_dropped_by_default() {
    // Tyrano: face = split(":")[1]; the ":c" part is lost.
    assert_eq!(
        lex("#a:b:c"),
        vec![Sharp, text("a"), Colon, text("b")]
    );
}

#[test]
fn chara_face_segments_can_be_preserved() {
    let cfg = ParserConfig {
        chara_face_first_segment_only: false,
        ..ParserConfig::default()
    };
    assert_eq!(
        lex_cfg("#a:b:c", cfg),
        vec![Sharp, text("a"), Colon, text("b:c")]
    );
}

#[test]
fn chara_segments_are_not_trimmed_individually() {
    // Tyrano only trims the whole line, so "#a: b" keeps the space in face.
    assert_eq!(
        lex("#a: b"),
        vec![Sharp, text("a"), Colon, text(" b")]
    );
}

// ---------------------------------------------------------------------------
// Tags: bracket and @ notation
// ---------------------------------------------------------------------------

#[test]
fn multi_param_tag_with_unquoted_values() {
    assert_eq!(
        lex("[bg storage=room.jpg time=1000]"),
        vec![
            LBracket,
            ident("bg"),
            ident("storage"),
            Equal,
            text("room.jpg"),
            ident("time"),
            Equal,
            num("1000"),
            RBracket,
        ]
    );
}

#[test]
fn quoted_value_keeps_spaces() {
    assert_eq!(
        lex(r#"[ptext text="Hello World" size=20]"#),
        vec![
            LBracket,
            ident("ptext"),
            ident("text"),
            Equal,
            string("Hello World"),
            ident("size"),
            Equal,
            num("20"),
            RBracket,
        ]
    );
}

#[test]
fn single_and_back_quotes_are_string_delimiters() {
    assert_eq!(
        lex(r#"[a v=`x "y"` w='z']"#),
        vec![
            LBracket,
            ident("a"),
            ident("v"),
            Equal,
            string(r#"x "y""#),
            ident("w"),
            Equal,
            string("z"),
            RBracket,
        ]
    );
}

#[test]
fn at_tag_consumes_the_whole_line() {
    assert_eq!(
        lex("@bg storage=\"title.jpg\" time=100"),
        vec![
            At,
            ident("bg"),
            ident("storage"),
            Equal,
            string("title.jpg"),
            ident("time"),
            Equal,
            num("100"),
        ]
    );
}

#[test]
fn spaces_around_equal_are_tolerated() {
    // makeTag's SCANNING_EQUAL / SCANNING_START_QUOT states skip spaces.
    assert_eq!(
        lex("@wait time = 200"),
        vec![At, ident("wait"), ident("time"), Equal, num("200")]
    );
    assert_eq!(
        lex("@wait time= 200"),
        vec![At, ident("wait"), ident("time"), Equal, num("200")]
    );
    assert_eq!(
        lex("[bg storage =room.jpg]"),
        vec![
            LBracket,
            ident("bg"),
            ident("storage"),
            Equal,
            text("room.jpg"),
            RBracket,
        ]
    );
}

#[test]
fn valueless_params_and_macro_star() {
    assert_eq!(
        lex("[macro_use * flag2]"),
        vec![LBracket, ident("macro_use"), Asterisk, ident("flag2"), RBracket]
    );
}

#[test]
fn flag_param_before_a_valued_param() {
    // [bg time storage=x] -> time flag param, storage=x.
    assert_eq!(
        lex("[bg time storage=x]"),
        vec![
            LBracket,
            ident("bg"),
            ident("time"),
            ident("storage"),
            Equal,
            text("x"),
            RBracket,
        ]
    );
}

#[test]
fn tag_name_may_start_with_a_digit() {
    // e.g. the official 3D plugin tags: [3d_init], [3d_model_new], ...
    assert_eq!(lex("[3d_init]"), vec![LBracket, ident("3d_init"), RBracket]);
}

#[test]
fn tag_name_after_leading_spaces() {
    assert_eq!(
        lex("[ bg time=100]"),
        vec![LBracket, ident("bg"), ident("time"), Equal, num("100"), RBracket]
    );
}

#[test]
fn nested_brackets_in_values() {
    // exp=f.a[0] relies on bracket-depth tracking, text="[[あ]]" on quote
    // tracking (both straight from parseScenario).
    assert_eq!(
        lex(r#"[ptext exp=f.a[0] text="[[あ]]"]"#),
        vec![
            LBracket,
            ident("ptext"),
            ident("exp"),
            Equal,
            text("f.a[0]"),
            ident("text"),
            Equal,
            string("[[あ]]"),
            RBracket,
        ]
    );
}

#[test]
fn unclosed_quote_is_compensated_like_tyrano() {
    // parseScenario: tag_str ends with "]" while a quote is open -> the "]"
    // is stripped ("compensate_missing_quart") and the tag is completed.
    assert_eq!(
        lex(r#"[ptext text="abc]"#),
        vec![
            LBracket,
            ident("ptext"),
            ident("text"),
            Equal,
            string("abc"),
            RBracket,
        ]
    );
}

#[test]
fn unclosed_tag_is_completed_at_end_of_line() {
    assert_eq!(
        lex("[ptext text=abc"),
        vec![
            LBracket,
            ident("ptext"),
            ident("text"),
            Equal,
            text("abc"),
            RBracket,
        ]
    );
}

#[test]
fn entity_and_param_ref_values() {
    assert_eq!(
        lex("[eval exp=&f.name]"),
        vec![
            LBracket,
            ident("eval"),
            ident("exp"),
            Equal,
            TokenType::Entity("&f.name".to_string()),
            RBracket,
        ]
    );
    assert_eq!(
        lex("[image storage=%img_file]"),
        vec![
            LBracket,
            ident("image"),
            ident("storage"),
            Equal,
            TokenType::ParamRef("%img_file".to_string()),
            RBracket,
        ]
    );
}

#[test]
fn empty_value_after_equal() {
    assert_eq!(
        lex("[a t=]"),
        vec![LBracket, ident("a"), ident("t"), Equal, RBracket]
    );
}

#[test]
fn undefined_value_becomes_empty_string() {
    // makeParam converts a trimmed "undefined" into "".
    assert_eq!(
        lex(r#"[a t="undefined"]"#),
        vec![LBracket, ident("a"), ident("t"), Equal, string(""), RBracket]
    );
}

// ---------------------------------------------------------------------------
// KeepSpaceInParameterValue levels
// ---------------------------------------------------------------------------

#[test]
fn keep_space_level_trim_ends_is_the_default() {
    assert_eq!(
        lex(r#"[a t=" x y "]"#),
        vec![LBracket, ident("a"), ident("t"), Equal, string("x y"), RBracket]
    );
}

#[test]
fn keep_space_level_remove_all() {
    let cfg = ParserConfig {
        keep_space_in_parameter_value: KeepSpaceLevel::RemoveAll,
        ..ParserConfig::default()
    };
    assert_eq!(
        lex_cfg(r#"[a t=" x y "]"#, cfg),
        vec![LBracket, ident("a"), ident("t"), Equal, string("xy"), RBracket]
    );
}

#[test]
fn keep_space_level_remove_all_spares_backquote_values() {
    let cfg = ParserConfig {
        keep_space_in_parameter_value: KeepSpaceLevel::RemoveAll,
        ..ParserConfig::default()
    };
    assert_eq!(
        lex_cfg("[a t=` x y `]", cfg),
        vec![LBracket, ident("a"), ident("t"), Equal, string("x y"), RBracket]
    );
}

#[test]
fn keep_space_level_keep_all() {
    let cfg = ParserConfig {
        keep_space_in_parameter_value: KeepSpaceLevel::KeepAll,
        ..ParserConfig::default()
    };
    assert_eq!(
        lex_cfg(r#"[a t=" x y "]"#, cfg),
        vec![LBracket, ident("a"), ident("t"), Equal, string(" x y "), RBracket]
    );
}

// ---------------------------------------------------------------------------
// iscript / html blocks
// ---------------------------------------------------------------------------

#[test]
fn iscript_block_basic() {
    assert_eq!(
        lex("[iscript]\nvar a = 1;\n[endscript]"),
        vec![
            IscriptStart,
            Newline,
            script("var a = 1;"),
            Newline,
            IscriptEnd,
        ]
    );
}

#[test]
fn loose_endscript_substring_terminates_the_block_by_default() {
    // 悪法: parseScenario exits script mode for ANY line containing the
    // substring "endscript" and then parses that line normally.
    assert_eq!(
        lex("[iscript]\nvar s = \"endscript\";\n[s]"),
        vec![
            IscriptStart,
            Newline,
            IscriptEnd,
            text("var s = \"endscript\";"),
            Newline,
            LBracket,
            ident("s"),
            RBracket,
        ]
    );
}

#[test]
fn strict_endscript_mode_keeps_script_text() {
    let cfg = ParserConfig {
        loose_endscript_termination: false,
        ..ParserConfig::default()
    };
    assert_eq!(
        lex_cfg("[iscript]\nvar s = \"endscript\";\n[endscript]", cfg),
        vec![
            IscriptStart,
            Newline,
            script("var s = \"endscript\";"),
            Newline,
            IscriptEnd,
        ]
    );
}

#[test]
fn unterminated_iscript_is_closed_at_eof() {
    assert_eq!(
        lex("[iscript]\nvar a = 1;"),
        vec![IscriptStart, Newline, script("var a = 1;"), IscriptEnd]
    );
}

#[test]
fn strict_mode_requires_the_exact_endscript_tag_name() {
    // A tag that merely shares the prefix ([endscript2]) must NOT end the
    // block in strict mode — its makeTag name is "endscript2".
    let cfg = ParserConfig {
        loose_endscript_termination: false,
        ..ParserConfig::default()
    };
    assert_eq!(
        lex_cfg("[iscript]\n[endscript2]\n@endscript2\n[endscript]", cfg),
        vec![
            IscriptStart,
            Newline,
            script("[endscript2]"),
            Newline,
            script("@endscript2"),
            Newline,
            IscriptEnd,
        ]
    );
}

#[test]
fn loose_mode_dispatches_prefixed_endscript_tag_as_a_normal_line() {
    // [endscript2] contains "endscript" so it ends the block (悪法), but the
    // engine then parses the line normally: it must surface as an ordinary
    // [endscript2] tag, not be swallowed as the closing tag.
    assert_eq!(
        lex("[iscript]\n[endscript2]"),
        vec![
            IscriptStart,
            Newline,
            IscriptEnd,
            LBracket,
            ident("endscript2"),
            RBracket,
        ]
    );
}

#[test]
fn endscript_with_params_still_ends_the_block() {
    // makeTag's name for "[endscript foo=1]" is "endscript".
    assert_eq!(
        lex("[iscript]\nvar a = 1;\n[endscript foo=1]"),
        vec![
            IscriptStart,
            Newline,
            script("var a = 1;"),
            Newline,
            IscriptEnd,
        ]
    );
}

#[test]
fn endscript_tag_with_leading_space_ends_the_block() {
    // makeTag skips spaces before the tag name, so "[ endscript]" is a real
    // endscript tag.
    assert_eq!(
        lex("[iscript]\n[ endscript]"),
        vec![IscriptStart, Newline, IscriptEnd]
    );
}

#[test]
fn html_block_basic() {
    assert_eq!(
        lex("[html]\n<b>あ</b>\n[endhtml]"),
        vec![HtmlStart, Newline, html("<b>あ</b>"), Newline, HtmlEnd]
    );
}

#[test]
fn prefixed_tag_names_do_not_start_blocks() {
    // [iscript2] / [html2] are ordinary tags; only the exact names switch
    // the scanner into script/html mode.
    assert_eq!(
        lex("[iscript2][html2]"),
        vec![
            LBracket,
            ident("iscript2"),
            RBracket,
            LBracket,
            ident("html2"),
            RBracket,
        ]
    );
}

#[test]
fn escaped_iscript_tag_is_plain_text() {
    // The engine's escape flag stops "[" from opening a tag, so no script
    // mode is entered.
    assert_eq!(lex(r"\[iscript]あ"), vec![text("[iscript]あ")]);
}

#[test]
fn html_block_is_not_ended_by_prefixed_tag_names() {
    // [endhtml2] / @endhtml2 are different tags; only a real endhtml tag
    // closes the block.
    assert_eq!(
        lex("[html]\n[endhtml2]\n@endhtml2\n[endhtml]"),
        vec![
            HtmlStart,
            Newline,
            html("[endhtml2]"),
            Newline,
            html("@endhtml2"),
            Newline,
            HtmlEnd,
        ]
    );
}

// ---------------------------------------------------------------------------
// Newlines and whole-file shape
// ---------------------------------------------------------------------------

#[test]
fn every_physical_newline_produces_a_newline_token() {
    assert_eq!(
        lex("[cm]\n\nこんにちは\n"),
        vec![
            LBracket,
            ident("cm"),
            RBracket,
            Newline,
            Newline,
            text("こんにちは"),
            Newline,
        ]
    );
}

#[test]
fn crlf_line_endings_are_handled() {
    assert_eq!(
        lex("*start\r\nこんにちは\r\n"),
        vec![Asterisk, text("start"), Newline, text("こんにちは"), Newline]
    );
}
