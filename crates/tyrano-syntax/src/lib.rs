//! Lossless, error-tolerant syntax tree infrastructure for TyranoScript.
//!
//! Pipeline: source → token/trivia stream → lossless CST (immutable green
//! tree + red cursor API) → typed AST views → semantic model (see the
//! `tyrano-analysis` crate).
//!
//! Design invariants:
//! - The CST is *full fidelity*: concatenating the tree's tokens and their
//!   trivia reproduces the input byte-for-byte, including whitespace,
//!   comments, BOM, escapes, and invalid characters.
//! - Parsing never fails: invalid input yields a tree containing
//!   `ERROR` nodes, missing tokens, and skipped tokens, plus structured
//!   diagnostics kept *outside* the tree.
//! - Engine compatibility quirks are interpretations, not mutations: the
//!   tree stores raw source; cooked values are computed by the AST view
//!   layer under [`ast::InterpretOptions`].

pub mod ast;
pub mod diagnostics;
pub mod expr;
pub mod green;
pub mod incremental;
pub mod kind;
pub mod lexer;
pub mod parser;
pub mod red;
pub mod text;
pub mod validation;

pub use kind::SyntaxKind;
pub use parser::{Parse, ParseOptions, parse, parse_with_options};
