//! TyranoScript lexer
//!
//! Runtime lexer shared by the generated `tyrano-parser` crate and the
//! `tyrano-parser-generator` tooling. This crate has no dependency on the
//! generator so that generated parsers stay free of build-tool code.

pub mod config;
pub mod scanner;
pub mod token;

pub use config::ParserConfig;
pub use scanner::Scanner;
pub use token::{Token, TokenType};

use thiserror::Error;

#[derive(Error, Debug)]
pub enum LexerError {
    #[error("Lexical error: {0}")]
    LexicalError(String),
}

pub type Result<T> = std::result::Result<T, LexerError>;
