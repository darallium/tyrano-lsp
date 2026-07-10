use std::fs;
use tyrano_lexer::Scanner;
use tyrano_parser::parser::{
    Parser,
};
use tyrano_parser::cst::CstNode;
use tyrano_parser::grammar_meta;
use tyrano_parser::tables::token_type_to_id;



pub fn run_tree(file_path: &str) -> Result<(), String> {
    let content =
        fs::read_to_string(file_path).map_err(|e| format!("Failed to read file: {}", e))?;

    println!("Parsing: {}", file_path);
    println!("---");

    // Tokenize using the lexer from tyrano-lexer
    let mut scanner = Scanner::new(&content);
    let gen_tokens = scanner
        .scan_tokens()
        .map_err(|e| format!("Lexer error: {}", e))?;

    // Convert tokens to parser's Token type
    let mut parser_tokens = Vec::new();
    let mut pos = 0usize;

    for gen_token in gen_tokens {
        let symbol_id = token_type_to_id(&gen_token.token_type);
        let lexeme = gen_token.lexeme().to_string();
        let end_pos = pos + lexeme.len();

        parser_tokens.push(tyrano_parser::parser::Token::new(
            symbol_id, lexeme, pos, end_pos,
        ));

        pos = end_pos;
    }

    // Parse
    match Parser::parse(parser_tokens) {
        Ok(cst) => {
            println!();
            print_s_expression(&cst, 0);
            println!();
        }
        Err(e) => {
            return Err(format!(
                "Parse error: {} at {}..{}\nExpected: {:?}",
                e.message, e.span.start, e.span.end, e.expected
            ));
        }
    }

    Ok(())
}

/// Print CST in S-expression format
fn print_s_expression(node: &CstNode, indent: usize) {
    let pad = "  ".repeat(indent);
    match node {
        CstNode::Terminal {
            symbol_id, lexeme, ..
        } => {
            let name = terminal_name(symbol_id.0);
            let display_lexeme = escape_lexeme(lexeme);
            println!("{}({} {:?})", pad, name, display_lexeme);
        }
        CstNode::NonTerminal {
            symbol_id,
            children,
            ..
        } => {
            let name = non_terminal_name(symbol_id.0);
            if children.is_empty() {
                println!("{}({})", pad, name);
            } else {
                println!("{}({}", pad, name);
                for child in children {
                    print_s_expression(child, indent + 1);
                }
                println!("{})", pad);
            }
        }
    }
}

/// Get the name of a terminal symbol by its ID
fn terminal_name(id: u32) -> &'static str {
    grammar_meta::get_symbol(id)
        .map(|s| s.name)
        .unwrap_or("UNKNOWN")
}

/// Get the name of a non-terminal symbol by its ID
fn non_terminal_name(id: u32) -> &'static str {
    grammar_meta::get_symbol(id)
        .map(|s| s.name)
        .unwrap_or("unknown")
}

/// Escape special characters in lexeme for display
fn escape_lexeme(s: &str) -> String {
    s.replace('\n', "\\n")
        .replace('\r', "\\r")
        .replace('\t', "\\t")
}

