use serde::{Deserialize, Serialize};
use std::borrow::Cow;

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum TokenType {
    // Literals
    Text(String),
    Identifier(String),
    Number(String),
    String(String),
    LineComment(String),

    // IScript mode
    IscriptStart,       // [iscript] detected - enter script mode
    IscriptEnd,         // [endscript] detected - exit script mode
    ScriptText(String), // Raw script content (non-newline)

    // HTML mode
    HtmlStart,        // [html] detected - enter html mode
    HtmlEnd,          // [endhtml] detected - exit html mode
    HtmlText(String), // Raw HTML content (non-newline)

    // Comments
    BlockCommentStart,
    BlockCommentEnd,

    // Entity reference (e.g. &tf.fileName)
    Entity(String),

    // Parameter reference (e.g. %bg, %param|default)
    ParamRef(String),

    // Special characters
    Sharp,      // #
    Asterisk,   // *
    At,         // @
    LBracket,   // [
    RBracket,   // ]
    Underscore, // _
    Equal,      // =
    Colon,      // :
    Pipe,       // |

    // Control
    Newline,
    Eof,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Token {
    pub token_type: TokenType,
    pub line: usize,
    pub column: usize,
}

impl Token {
    pub fn new(token_type: TokenType, line: usize, column: usize) -> Self {
        Token {
            token_type,
            line,
            column,
        }
    }

    /// Returns the textual representation associated with this token.
    pub fn lexeme(&self) -> Cow<'_, str> {
        match &self.token_type {
            TokenType::Text(value)
            | TokenType::Identifier(value)
            | TokenType::String(value)
            | TokenType::LineComment(value)
            | TokenType::ScriptText(value)
            | TokenType::HtmlText(value)
            | TokenType::Entity(value)
            | TokenType::ParamRef(value) => Cow::Borrowed(value.as_str()),
            TokenType::IscriptStart => Cow::Borrowed("[iscript]"),
            TokenType::IscriptEnd => Cow::Borrowed("[endscript]"),
            TokenType::HtmlStart => Cow::Borrowed("[html]"),
            TokenType::HtmlEnd => Cow::Borrowed("[endhtml]"),
            TokenType::Number(value) => Cow::Owned(value.to_string()),
            TokenType::BlockCommentStart => Cow::Borrowed("/*"),
            TokenType::BlockCommentEnd => Cow::Borrowed("*/"),
            TokenType::Sharp => Cow::Borrowed("#"),
            TokenType::Asterisk => Cow::Borrowed("*"),
            TokenType::At => Cow::Borrowed("@"),
            TokenType::LBracket => Cow::Borrowed("["),
            TokenType::RBracket => Cow::Borrowed("]"),
            TokenType::Underscore => Cow::Borrowed("_"),
            TokenType::Equal => Cow::Borrowed("="),
            TokenType::Colon => Cow::Borrowed(":"),
            TokenType::Pipe => Cow::Borrowed("|"),
            TokenType::Newline => Cow::Borrowed("\n"),
            TokenType::Eof => Cow::Borrowed(""),
        }
    }
}
