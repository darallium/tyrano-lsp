use std::fs;
use tyrano_lexer::Scanner;
use tyrano_parser::{
    parser::{ Parser,},
    cst::CstNode,
    tables::token_type_to_id,
};


pub fn run_parse(file_path: &str) -> Result<(), String> {
    let content = fs::read_to_string(file_path).expect("Failed to read file");

    println!("Parsing: {}", file_path);
    println!("Content length: {} bytes", content.len());
    println!("---");

    // Tokenize using the lexer from tyrano-lexer
    let mut scanner = Scanner::new(&content);
    let tokens_result = scanner.scan_tokens();

    match tokens_result {
        Ok(gen_tokens) => {
            println!("Tokenized: {} tokens", gen_tokens.len());

            // Debug: Print first 50 tokens to see what's happening
            println!("\n=== Token Debug ===");
            for (i, tok) in gen_tokens.iter().enumerate() {
                let lexeme = tok.lexeme();
                let display = if lexeme == "\n" {
                    "\\n".to_string()
                } else {
                    lexeme.to_string()
                };
                println!("  [{:3}] {:?} = {:?}", i, tok.token_type, display);
            }
            println!("=== End Token Debug ===\n");

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
                    println!("\n=== Parse Successful ===\n");
                    print_cst(&cst, 0);
                    Ok(())
                }
                Err(e) => {
                    if e.expected.is_empty() {
                        return Err(format!(
                            "\n=== Parse error ===\n{}\nAt position: {}..{}",
                            e.message, e.span.start, e.span.end
                        ));
                    } else {
                        return Err(format!(
                            "\n=== Parse error ===\n{}\nAt position: {}..{}\nExpected: {:?}",
                            e.message, e.span.start, e.span.end, e.expected
                        ));
                    }
                }
            }
        }
        Err(e) => {
            Err(format!("Lexer error: {}", e))
        }
    }
}



fn print_cst(node: &CstNode, indent: usize) {
    let pad = "  ".repeat(indent);
    match node {
        CstNode::Terminal {
            symbol_id,
            lexeme,
            span,
        } => {
            let display_lexeme = if lexeme == "\n" {
                "\\n".to_string()
            } else {
                lexeme.clone()
            };
            println!(
                "{}Terminal(sym={}, lexeme={:?}, {}..{})",
                pad, symbol_id.0, display_lexeme, span.start, span.end
            );
        }
        CstNode::NonTerminal {
            production_id,
            symbol_id,
            children,
            span,
        } => {
            println!(
                "{}NonTerminal(prod={}, sym={}, {}..{}) [",
                pad, production_id.0, symbol_id.0, span.start, span.end
            );
            for child in children {
                print_cst(child, indent + 1);
            }
            println!("{}]", pad);
        }
    }
}


