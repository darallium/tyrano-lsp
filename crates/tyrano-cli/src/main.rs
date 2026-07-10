//! CLI for the lossless TyranoScript syntax stack (`tyrano-syntax` +
//! `tyrano-analysis`).
//!
//! Unlike the legacy `tyrano-parser-debugger`, every command here operates
//! on the full-fidelity CST: token dumps include trivia, tree dumps carry
//! real byte ranges, and `roundtrip` proves the tree reproduces the input
//! byte-for-byte.

use std::fmt::Write as _;
use std::path::PathBuf;
use std::process::ExitCode;

use clap::{Parser as ClapParser, Subcommand};
use tyrano_analysis::Item;
use tyrano_syntax::ast::InterpretOptions;
use tyrano_syntax::diagnostics::{Lang, Severity, render_with_location};
use tyrano_syntax::lexer::{LexOptions, lex};
use tyrano_syntax::red::{SyntaxElement, SyntaxNode};
use tyrano_syntax::text::TextSize;
use tyrano_syntax::{Parse, ParseOptions, parse_with_options};

#[derive(ClapParser)]
#[command(name = "tyrano-cli", about = "Lossless TyranoScript parser toolkit")]
struct Cli {
    /// Disable the engine's loose-endscript quirk (strict block endings).
    #[arg(long, global = true)]
    strict: bool,
    /// Diagnostic message language.
    #[arg(long, global = true, default_value = "en", value_parser = ["en", "ja"])]
    lang: String,
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Dump the flat token/trivia stream (every byte accounted for).
    Tokens { file: PathBuf },
    /// Dump the lossless syntax tree (optionally with trivia detail).
    Tree {
        file: PathBuf,
        /// Also print each token's leading/trailing trivia pieces.
        #[arg(long)]
        trivia: bool,
    },
    /// Print rendered diagnostics; exits non-zero on errors.
    Diag { file: PathBuf },
    /// Verify the byte-exact round-trip invariant.
    Roundtrip { file: PathBuf },
    /// Dump the typed AST view with cooked values.
    Ast { file: PathBuf },
    /// Parse one embedded expression (Pratt sub-parser) and dump its tree.
    Expr { expression: String },
    /// Lower to the semantic model: items + semantic diagnostics.
    Analyze { file: PathBuf },
}

fn main() -> ExitCode {
    let cli = Cli::parse();
    match run(&cli) {
        Ok(code) => code,
        Err(msg) => {
            eprintln!("error: {msg}");
            ExitCode::FAILURE
        }
    }
}

fn run(cli: &Cli) -> Result<ExitCode, String> {
    let opts = ParseOptions { loose_endscript_termination: !cli.strict };
    let lang = if cli.lang == "ja" { Lang::Ja } else { Lang::En };

    match &cli.command {
        Command::Tokens { file } => {
            let source = read(file)?;
            let lexed = lex(
                &source,
                &LexOptions { loose_endscript_termination: !cli.strict },
            );
            let mut pos = 0usize;
            for tok in &lexed.tokens {
                let end = pos + tok.len.raw() as usize;
                println!("{pos:>5}..{end:<5} {:<14} {}", tok.kind.to_string(), preview(&source[pos..end]));
                pos = end;
            }
            println!("-- {} tokens, {} bytes covered", lexed.tokens.len(), pos);
            Ok(ExitCode::SUCCESS)
        }
        Command::Tree { file, trivia } => {
            let source = read(file)?;
            let parsed = parse_with_options(&source, &opts);
            print!("{}", tree_dump(&SyntaxNode::new_root(parsed.green().clone()), *trivia));
            Ok(ExitCode::SUCCESS)
        }
        Command::Diag { file } => {
            let source = read(file)?;
            let parsed = parse_with_options(&source, &opts);
            let mut errors = 0usize;
            let mut warnings = 0usize;
            for d in parsed.diagnostics() {
                match d.severity {
                    Severity::Error => errors += 1,
                    Severity::Warning => warnings += 1,
                    Severity::Info => {}
                }
                println!("{}", render_with_location(d, lang, parsed.source()));
            }
            println!("{errors} error(s), {warnings} warning(s)");
            Ok(if errors > 0 { ExitCode::FAILURE } else { ExitCode::SUCCESS })
        }
        Command::Roundtrip { file } => {
            let source = read(file)?;
            let parsed = parse_with_options(&source, &opts);
            let rebuilt = parsed.to_source();
            if rebuilt == source {
                println!("OK {} bytes", source.len());
                Ok(ExitCode::SUCCESS)
            } else {
                let at = source
                    .bytes()
                    .zip(rebuilt.bytes())
                    .position(|(a, b)| a != b)
                    .unwrap_or(source.len().min(rebuilt.len()));
                Err(format!(
                    "round-trip diverged at byte {at}: input {:?} vs rebuilt {:?}",
                    window(&source, at),
                    window(&rebuilt, at),
                ))
            }
        }
        Command::Ast { file } => {
            let source = read(file)?;
            let parsed = parse_with_options(&source, &opts);
            print!("{}", ast_dump(&parsed));
            Ok(ExitCode::SUCCESS)
        }
        Command::Expr { expression } => {
            let result = tyrano_syntax::expr::parse_expr(expression, TextSize::new(0));
            print!("{}", tree_dump(&result.syntax(), false));
            for d in result.diagnostics() {
                println!("{} @{:?}", d.code.as_str(), d.primary);
            }
            Ok(ExitCode::SUCCESS)
        }
        Command::Analyze { file } => {
            let source = read(file)?;
            let parsed = parse_with_options(&source, &opts);
            let model = tyrano_analysis::lower(&parsed, &InterpretOptions::default());
            for item in model.items() {
                println!("{}", item_line(item));
            }
            for d in model.diagnostics() {
                println!("{}", render_with_location(d, lang, parsed.source()));
            }
            println!(
                "-- {} item(s), {} label(s), {} semantic diagnostic(s)",
                model.items().len(),
                model.labels().count(),
                model.diagnostics().len()
            );
            Ok(if model.diagnostics().is_empty() {
                ExitCode::SUCCESS
            } else {
                ExitCode::FAILURE
            })
        }
    }
}

fn read(path: &PathBuf) -> Result<String, String> {
    std::fs::read_to_string(path).map_err(|e| format!("cannot read {}: {e}", path.display()))
}

/// `{:?}`-escaped text preview, truncated for sanity.
fn preview(text: &str) -> String {
    let escaped = format!("{text:?}");
    if escaped.chars().count() > 44 {
        let cut: String = escaped.chars().take(40).collect();
        format!("{cut}…\"")
    } else {
        escaped
    }
}

fn window(s: &str, at: usize) -> String {
    let start = at.saturating_sub(10);
    let end = (at + 10).min(s.len());
    let (mut a, mut b) = (start, end);
    while !s.is_char_boundary(a) {
        a -= 1;
    }
    while !s.is_char_boundary(b) {
        b += 1;
    }
    s[a..b].to_string()
}

/// Shared recursive tree printer (used by `tree` and `expr`).
fn tree_dump(root: &SyntaxNode, trivia: bool) -> String {
    let mut out = String::new();
    dump_node(root, 0, trivia, &mut out);
    out
}

fn dump_node(node: &SyntaxNode, depth: usize, trivia: bool, out: &mut String) {
    let pad = "  ".repeat(depth);
    let _ = writeln!(out, "{pad}{}@{:?}", node.kind(), node.text_range());
    for el in node.children_with_tokens() {
        match el {
            SyntaxElement::Node(n) => dump_node(&n, depth + 1, trivia, out),
            SyntaxElement::Token(t) => {
                let pad = "  ".repeat(depth + 1);
                let _ = write!(out, "{pad}{}@{:?} {}", t.kind(), t.text_range(), preview(t.text()));
                if t.is_missing() {
                    let _ = write!(out, " (missing)");
                }
                if trivia {
                    for (kind, range) in t.leading_trivia_ranges() {
                        let _ = write!(out, " lead({kind}@{range:?})");
                    }
                    for (kind, range) in t.trailing_trivia_ranges() {
                        let _ = write!(out, " trail({kind}@{range:?})");
                    }
                }
                out.push('\n');
            }
        }
    }
}

/// One-line-per-construct dump of the typed AST view (cooked values).
fn ast_dump(parsed: &Parse) -> String {
    use tyrano_syntax::ast::Line;
    let io = InterpretOptions::default();
    let mut out = String::new();
    for line in parsed.ast().lines() {
        match &line {
            Line::Text(t) => {
                let _ = writeln!(
                    out,
                    "Text preserve={} {}",
                    t.preserves_whitespace(),
                    preview(&t.cooked_text())
                );
            }
            Line::Label(l) => {
                let _ = writeln!(out, "Label name={:?} value={:?}", l.name(), l.value(&io));
            }
            Line::Chara(c) => {
                let _ = writeln!(out, "Chara name={:?} face={:?}", c.name(), c.face(&io));
            }
            Line::Comment(c) => {
                let _ = writeln!(out, "Comment {}", preview(&c.text().unwrap_or_default()));
            }
            Line::BlockComment(b) => {
                let _ = writeln!(out, "BlockComment {} line(s)", b.text_lines().len());
            }
            Line::AtTag(t) => {
                let _ = writeln!(out, "AtTag {}", tag_desc(t, &io));
            }
            Line::IScript(s) => {
                let _ = writeln!(out, "Script {} line(s)", s.code().lines().count());
            }
            Line::Html(h) => {
                let _ = writeln!(out, "Html {} line(s)", h.code().lines().count());
            }
            Line::Error(e) => {
                use tyrano_syntax::ast::AstNode as _;
                let _ = writeln!(out, "Error @{:?}", e.syntax().text_range());
            }
        }
        if let Line::Text(t) = &line {
            for seg in t.segments() {
                if let tyrano_syntax::ast::TextSegment::Tag(tag) = seg {
                    let _ = writeln!(out, "  InlineTag {}", tag_desc(&tag, &io));
                }
            }
        }
    }
    out
}

fn tag_desc(tag: &impl tyrano_syntax::ast::Tag, io: &InterpretOptions) -> String {
    let params: Vec<String> = tag
        .params()
        .iter()
        .map(|p| match p.cooked_value(io) {
            Some(v) => format!("{}={v:?}", p.name()),
            None => p.name(),
        })
        .collect();
    format!("{} [{}]", tag.name(), params.join(" "))
}

fn item_line(item: &Item) -> String {
    match item {
        Item::Label(l) => format!("label   {:?} value={:?} @{:?}", l.name, l.value, l.range),
        Item::Tag(t) => format!(
            "tag     {} {:?}{} @{:?}",
            t.name,
            t.params,
            if t.at_notation { " (@)" } else { "" },
            t.range
        ),
        Item::Chara(c) => format!("chara   {:?}:{:?} @{:?}", c.name, c.face, c.range),
        Item::Text(t) => format!("text    {} @{:?}", preview(&t.text), t.range),
        Item::Comment(c) => {
            format!("comment {} block={} @{:?}", preview(&c.text), c.is_block, c.range)
        }
        Item::Script(s) => format!("script  {} line(s) @{:?}", s.code.lines().count(), s.range),
        Item::Html(h) => format!("html    {} line(s) @{:?}", h.content.lines().count(), h.range),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tree_dump_shows_structure_and_missing() {
        let parsed = tyrano_syntax::parse("[]\n");
        let dump = tree_dump(&SyntaxNode::new_root(parsed.green().clone()), false);
        assert!(dump.contains("inline_tag@0..2"));
        assert!(dump.contains("(missing)"));
    }

    #[test]
    fn preview_truncates() {
        let long = "あ".repeat(100);
        assert!(preview(&long).chars().count() < 50);
        assert_eq!(preview("ab"), "\"ab\"");
    }

    #[test]
    fn ast_dump_covers_lines() {
        let parsed = tyrano_syntax::parse("*a|b\n#n:f\nこんにちは[l]\n;c\n");
        let dump = ast_dump(&parsed);
        assert!(dump.contains("Label"));
        assert!(dump.contains("Chara"));
        assert!(dump.contains("InlineTag l"));
        assert!(dump.contains("Comment"));
    }
}
