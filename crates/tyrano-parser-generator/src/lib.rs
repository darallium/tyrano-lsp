pub mod ast;
pub mod cli;
pub mod codegen;
pub mod generator;
pub mod grammar;
pub mod parser;
pub mod state;
pub mod visualizer;

// Re-export codegen for main.rs
pub use codegen::ParserGeneratorCodegen;

use thiserror::Error;

#[derive(Error, Debug)]
pub enum ParserError {
    #[error(transparent)]
    LexicalError(#[from] tyrano_lexer::LexerError),

    #[error("Syntax error: {0}")]
    SyntaxError(String),

    #[error("Grammar error: {0}")]
    GrammarError(String),

    #[error("State error: {0}")]
    StateError(String),
}

pub type Result<T> = std::result::Result<T, ParserError>;
