//! The single, authoritative vocabulary of the syntax tree.
//!
//! `SyntaxKind` names every token, trivia piece, and node that can appear in
//! the lossless TyranoScript CST, plus the kinds used by the embedded
//! expression sub-language. The discriminant ordering is part of the
//! contract: trivia kinds, then token kinds, then node kinds, delimited by
//! the `__*_START`/`__*_END` markers that back the classification helpers.

/// Kind tag for every element of the syntax tree.
///
/// One flat enum covers scenario syntax and the embedded expression
/// sub-language so that both share the same green/red tree infrastructure.
/// A few punctuation kinds are reused across the two layers (`STAR`, `EQ`,
/// `L_BRACKET`, `R_BRACKET`); the surrounding node kind disambiguates.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[repr(u16)]
#[allow(non_camel_case_types)]
pub enum SyntaxKind {
    /// Placeholder produced by builder checkpoints; never appears in a
    /// finished tree.
    TOMBSTONE = 0,
    /// End of file. Carries only trivia (e.g. trailing whitespace with no
    /// final newline is *not* EOF trivia — it belongs to the last token —
    /// but a lone BOM in an empty file lands here).
    EOF,

    // ------------------------------------------------------------------
    // Trivia (attached to tokens as leading/trailing pieces)
    // ------------------------------------------------------------------
    __TRIVIA_START,
    /// Horizontal whitespace (spaces, tabs). Never contains `\n`.
    WHITESPACE,
    /// A UTF-8 byte-order mark at offset 0.
    BOM,
    __TRIVIA_END,

    // ------------------------------------------------------------------
    // Tokens — scenario layer
    // ------------------------------------------------------------------
    __TOKEN_START,
    /// A line terminator: `\n` or `\r\n` (the token text preserves which).
    NEWLINE,
    /// Free text content of a text line (includes interior spaces and
    /// escape backslashes verbatim).
    TEXT,
    /// Tag or parameter name.
    IDENT,
    /// Unquoted numeric parameter value (digits with at most one interior dot).
    NUMBER,
    /// Quoted parameter value *including* its delimiter characters
    /// (`"…"`, `'…'`, or `` `…` ``) and any escape backslashes, verbatim.
    STRING,
    /// `&expr` entity reference used as a parameter value.
    ENTITY,
    /// `%name` macro parameter reference used as a parameter value.
    PARAM_REF,
    /// `@` introducing a whole-line tag.
    AT,
    /// `#` introducing a character-name line.
    SHARP,
    /// `*` introducing a label line, the macro pass-through parameter, or
    /// multiplication inside expressions.
    STAR,
    /// `[` opening an inline tag or an index expression.
    L_BRACKET,
    /// `]` closing an inline tag or an index expression.
    R_BRACKET,
    /// `_` at line start (whitespace-preserving text line marker).
    UNDERSCORE,
    /// `=` between a parameter name and value, or assignment in expressions.
    EQ,
    /// `:` between character name and face.
    COLON,
    /// `|` between label name and value.
    PIPE,
    /// `;` introducing a comment line.
    SEMICOLON,
    /// The body text of a `;` comment line or a block-comment interior line.
    COMMENT_TEXT,
    /// One raw line inside an `[iscript]` block.
    SCRIPT_TEXT,
    /// One raw line inside an `[html]` block.
    HTML_TEXT,
    /// A line consisting exactly of `/*` (block comment opener).
    SLASH_STAR,
    /// A line consisting exactly of `*/` (block comment closer).
    STAR_SLASH,

    // ------------------------------------------------------------------
    // Tokens — expression sub-language
    // ------------------------------------------------------------------
    PLUS,
    MINUS,
    SLASH,
    PERCENT,
    BANG,
    /// `&&`
    AMP2,
    /// `||`
    PIPE2,
    /// `==`
    EQ2,
    /// `===`
    EQ3,
    /// `!=`
    NEQ,
    /// `!==`
    NEQ2,
    LT,
    GT,
    LT_EQ,
    GT_EQ,
    QUESTION,
    DOT,
    COMMA,
    L_PAREN,
    R_PAREN,
    PLUS_EQ,
    MINUS_EQ,
    STAR_EQ,
    SLASH_EQ,
    PERCENT_EQ,
    TRUE_KW,
    FALSE_KW,
    NULL_KW,
    UNDEFINED_KW,
    TYPEOF_KW,

    /// A byte sequence the lexer could not assign any meaningful kind.
    /// Still carries its exact source text (losslessness holds).
    ERROR_TOKEN,
    __TOKEN_END,

    // ------------------------------------------------------------------
    // Nodes — scenario layer
    // ------------------------------------------------------------------
    __NODE_START,
    /// Root node: the whole `.ks` file.
    SCENARIO,
    /// A free-text line, possibly starting with `_` and mixing TEXT tokens
    /// with INLINE_TAG nodes.
    TEXT_LINE,
    /// `*name|value` line.
    LABEL_LINE,
    /// The name part of a label line.
    LABEL_NAME,
    /// The value part of a label line (everything after `|`, kept whole).
    LABEL_VALUE,
    /// `#name:face` line.
    CHARA_LINE,
    /// The name part of a character line.
    CHARA_NAME,
    /// The face part of a character line (everything after `:`, kept whole).
    CHARA_FACE,
    /// `;comment` line.
    COMMENT_LINE,
    /// A `/* … */` region including every interior line.
    BLOCK_COMMENT,
    /// `@tag param=value …` whole-line tag.
    AT_TAG_LINE,
    /// `[tag param=value …]` inline tag.
    INLINE_TAG,
    /// The tag-name inside AT_TAG_LINE / INLINE_TAG.
    TAG_NAME,
    /// One `name`, `name=`, `name=value`, or `*` parameter.
    PARAM,
    /// The value of a parameter (wraps STRING/NUMBER/TEXT/ENTITY/PARAM_REF).
    PARAM_VALUE,
    /// `[iscript] … [endscript]` block, including its opening/closing tag
    /// syntax and every raw script line.
    ISCRIPT_BLOCK,
    /// `[html] … [endhtml]` block.
    HTML_BLOCK,
    /// Syntactically invalid region; children are the skipped elements.
    ERROR,

    // ------------------------------------------------------------------
    // Nodes — expression sub-language
    // ------------------------------------------------------------------
    /// Root of an expression sub-parse (anchored at a PARAM_VALUE / ENTITY).
    EXPR_ROOT,
    LITERAL,
    NAME_REF,
    PAREN_EXPR,
    PREFIX_EXPR,
    /// Any binary operation, including assignment and comma sequencing;
    /// the operator token distinguishes them.
    BIN_EXPR,
    TERNARY_EXPR,
    CALL_EXPR,
    INDEX_EXPR,
    FIELD_EXPR,
    ARG_LIST,
    __NODE_END,
}

impl SyntaxKind {
    #[inline]
    pub const fn into_raw(self) -> u16 {
        self as u16
    }

    /// Reconstructs a kind from its raw discriminant.
    ///
    /// # Panics
    /// Panics if `raw` is not a valid discriminant.
    #[inline]
    pub const fn from_raw(raw: u16) -> SyntaxKind {
        assert!(raw <= SyntaxKind::__NODE_END as u16);
        // SAFETY: repr(u16) fieldless enum with contiguous discriminants
        // 0..=__NODE_END, and the range was just checked.
        unsafe { core::mem::transmute::<u16, SyntaxKind>(raw) }
    }

    /// Whitespace-like pieces that ride on tokens instead of standing alone.
    #[inline]
    pub const fn is_trivia(self) -> bool {
        (self as u16) > (SyntaxKind::__TRIVIA_START as u16)
            && (self as u16) < (SyntaxKind::__TRIVIA_END as u16)
    }

    /// True for leaf kinds (including `EOF` and `ERROR_TOKEN`).
    #[inline]
    pub const fn is_token(self) -> bool {
        matches!(self, SyntaxKind::EOF)
            || ((self as u16) > (SyntaxKind::__TOKEN_START as u16)
                && (self as u16) < (SyntaxKind::__TOKEN_END as u16))
    }

    /// True for interior-node kinds.
    #[inline]
    pub const fn is_node(self) -> bool {
        (self as u16) > (SyntaxKind::__NODE_START as u16)
            && (self as u16) < (SyntaxKind::__NODE_END as u16)
    }

    /// The line-level node kinds that can appear directly under `SCENARIO`.
    #[inline]
    pub const fn is_line(self) -> bool {
        matches!(
            self,
            SyntaxKind::TEXT_LINE
                | SyntaxKind::LABEL_LINE
                | SyntaxKind::CHARA_LINE
                | SyntaxKind::COMMENT_LINE
                | SyntaxKind::BLOCK_COMMENT
                | SyntaxKind::AT_TAG_LINE
                | SyntaxKind::ISCRIPT_BLOCK
                | SyntaxKind::HTML_BLOCK
                | SyntaxKind::ERROR
        )
    }

    /// Stable lowercase name used by debug dumps and golden files.
    pub const fn name(self) -> &'static str {
        use SyntaxKind::*;
        match self {
            TOMBSTONE => "tombstone",
            EOF => "eof",
            WHITESPACE => "whitespace",
            BOM => "bom",
            NEWLINE => "newline",
            TEXT => "text",
            IDENT => "ident",
            NUMBER => "number",
            STRING => "string",
            ENTITY => "entity",
            PARAM_REF => "param_ref",
            AT => "at",
            SHARP => "sharp",
            STAR => "star",
            L_BRACKET => "l_bracket",
            R_BRACKET => "r_bracket",
            UNDERSCORE => "underscore",
            EQ => "eq",
            COLON => "colon",
            PIPE => "pipe",
            SEMICOLON => "semicolon",
            COMMENT_TEXT => "comment_text",
            SCRIPT_TEXT => "script_text",
            HTML_TEXT => "html_text",
            SLASH_STAR => "slash_star",
            STAR_SLASH => "star_slash",
            PLUS => "plus",
            MINUS => "minus",
            SLASH => "slash",
            PERCENT => "percent",
            BANG => "bang",
            AMP2 => "amp2",
            PIPE2 => "pipe2",
            EQ2 => "eq2",
            EQ3 => "eq3",
            NEQ => "neq",
            NEQ2 => "neq2",
            LT => "lt",
            GT => "gt",
            LT_EQ => "lt_eq",
            GT_EQ => "gt_eq",
            QUESTION => "question",
            DOT => "dot",
            COMMA => "comma",
            L_PAREN => "l_paren",
            R_PAREN => "r_paren",
            PLUS_EQ => "plus_eq",
            MINUS_EQ => "minus_eq",
            STAR_EQ => "star_eq",
            SLASH_EQ => "slash_eq",
            PERCENT_EQ => "percent_eq",
            TRUE_KW => "true_kw",
            FALSE_KW => "false_kw",
            NULL_KW => "null_kw",
            UNDEFINED_KW => "undefined_kw",
            TYPEOF_KW => "typeof_kw",
            ERROR_TOKEN => "error_token",
            SCENARIO => "scenario",
            TEXT_LINE => "text_line",
            LABEL_LINE => "label_line",
            LABEL_NAME => "label_name",
            LABEL_VALUE => "label_value",
            CHARA_LINE => "chara_line",
            CHARA_NAME => "chara_name",
            CHARA_FACE => "chara_face",
            COMMENT_LINE => "comment_line",
            BLOCK_COMMENT => "block_comment",
            AT_TAG_LINE => "at_tag_line",
            INLINE_TAG => "inline_tag",
            TAG_NAME => "tag_name",
            PARAM => "param",
            PARAM_VALUE => "param_value",
            ISCRIPT_BLOCK => "iscript_block",
            HTML_BLOCK => "html_block",
            ERROR => "error",
            EXPR_ROOT => "expr_root",
            LITERAL => "literal",
            NAME_REF => "name_ref",
            PAREN_EXPR => "paren_expr",
            PREFIX_EXPR => "prefix_expr",
            BIN_EXPR => "bin_expr",
            TERNARY_EXPR => "ternary_expr",
            CALL_EXPR => "call_expr",
            INDEX_EXPR => "index_expr",
            FIELD_EXPR => "field_expr",
            ARG_LIST => "arg_list",
            __TRIVIA_START | __TRIVIA_END | __TOKEN_START | __TOKEN_END | __NODE_START
            | __NODE_END => "__marker",
        }
    }
}

impl core::fmt::Display for SyntaxKind {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str(self.name())
    }
}

#[cfg(test)]
mod tests {
    use super::SyntaxKind;

    #[test]
    fn raw_roundtrip() {
        for raw in 0..=SyntaxKind::__NODE_END.into_raw() {
            let kind = SyntaxKind::from_raw(raw);
            assert_eq!(kind.into_raw(), raw);
        }
    }

    #[test]
    fn classification_is_disjoint() {
        for raw in 0..=SyntaxKind::__NODE_END.into_raw() {
            let kind = SyntaxKind::from_raw(raw);
            let classes =
                [kind.is_trivia(), kind.is_token(), kind.is_node()].iter().filter(|&&b| b).count();
            assert!(classes <= 1, "{kind} belongs to multiple classes");
        }
        assert!(SyntaxKind::WHITESPACE.is_trivia());
        assert!(SyntaxKind::NEWLINE.is_token());
        assert!(SyntaxKind::ERROR_TOKEN.is_token());
        assert!(SyntaxKind::EOF.is_token());
        assert!(SyntaxKind::SCENARIO.is_node());
        assert!(SyntaxKind::ARG_LIST.is_node());
        assert!(SyntaxKind::ERROR.is_node());
        assert!(SyntaxKind::ERROR.is_line());
    }
}
