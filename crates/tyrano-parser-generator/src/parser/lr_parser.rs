use crate::Result;
use crate::ast::{AstNode, Parameter};
use crate::generator::{Action, ParseTable};
use crate::grammar::{GrammarParser, Production, Symbol};
use std::rc::Rc;
use tyrano_lexer::{ParserConfig, Scanner, Token, TokenType};

pub struct LRParser {
    parse_table: ParseTable,
    grammar: GrammarParser,
}

impl LRParser {
    pub fn new(parse_table: ParseTable, grammar: GrammarParser) -> Self {
        LRParser {
            parse_table,
            grammar,
        }
    }

    pub fn parse(&self, input: &str) -> Result<AstNode> {
        self.parse_with_config(input, ParserConfig::default())
    }

    pub fn parse_with_config(&self, input: &str, config: ParserConfig) -> Result<AstNode> {
        let mut scanner = Scanner::with_config(input, config);
        let tokens = scanner.scan_tokens()?;

        self.parse_tokens(tokens)
    }

    fn parse_tokens(&self, tokens: Vec<Token>) -> Result<AstNode> {
        let mut stack: Vec<usize> = vec![0]; // State stack
        let mut value_stack: Vec<AstNode> = Vec::new();
        let mut token_iter = tokens.into_iter();
        let mut current_token = token_iter.next();

        loop {
            let state = *stack.last().unwrap();
            let symbol = self.token_to_symbol(&current_token);

            match self.parse_table.action_table.get(&(state, symbol.clone())) {
                Some(Action::Shift(next_state)) => {
                    stack.push(*next_state);

                    // Create AST node for the token
                    if let Some(ref token) = current_token {
                        value_stack.push(self.create_terminal_node(token));
                    }

                    current_token = token_iter.next();
                }
                Some(Action::Reduce(production_id)) => {
                    let production = Rc::clone(&self.grammar.get_productions()[*production_id]);
                    let rhs_len = if production.is_epsilon_production() {
                        0
                    } else {
                        production.rhs.len()
                    };

                    // Pop states
                    for _ in 0..rhs_len {
                        stack.pop();
                    }

                    // Pop values and create new AST node
                    let mut children = Vec::new();
                    for _ in 0..rhs_len {
                        if let Some(node) = value_stack.pop() {
                            children.push(Box::new(node));
                        }
                    }
                    children.reverse();

                    let new_node = self.create_non_terminal_node(production.as_ref(), children);
                    value_stack.push(new_node);

                    // Goto
                    let state = *stack.last().unwrap();
                    if let Some(&goto_state) = self
                        .parse_table
                        .goto_table
                        .get(&(state, production.lhs.clone()))
                    {
                        stack.push(goto_state);
                    } else {
                        return Err(crate::ParserError::SyntaxError(format!(
                            "No goto for state {} and symbol {}",
                            state, production.lhs
                        )));
                    }
                }
                Some(Action::Accept) => {
                    return value_stack.pop().ok_or_else(|| {
                        crate::ParserError::SyntaxError("Empty value stack".to_string())
                    });
                }
                _ => {
                    let token_str = current_token
                        .as_ref()
                        .map(|t| format!("{:?}", t.token_type))
                        .unwrap_or_else(|| "EOF".to_string());

                    #[cfg(debug_assertions)]
                    {
                        // Debug: print available actions in this state
                        eprintln!("=== Debug: State {state} ===");
                        eprintln!("Current symbol: {symbol:?}");
                        eprintln!("Available actions in state {state}:");
                        for ((s, sym), action) in &self.parse_table.action_table {
                            if *s == state {
                                eprintln!("  {sym:?} -> {action:?}");
                            }
                        }
                    }

                    return Err(crate::ParserError::SyntaxError(format!(
                        "Unexpected token: {token_str} in state {state}"
                    )));
                }
            }
        }
    }

    fn token_to_symbol(&self, token: &Option<Token>) -> Symbol {
        match token {
            Some(token) => match &token.token_type {
                TokenType::Text(_) => Symbol::terminal("TEXT"),
                TokenType::Identifier(_) => Symbol::terminal("IDENTIFIER"),
                TokenType::Number(_) => Symbol::terminal("NUMBER"),
                TokenType::String(_) => Symbol::terminal("STRING"),
                TokenType::ScriptText(_) => Symbol::terminal("SCRIPT_TEXT"),
                TokenType::HtmlText(_) => Symbol::terminal("HTML_TEXT"),
                TokenType::IscriptStart => Symbol::terminal("ISCRIPT_START"),
                TokenType::IscriptEnd => Symbol::terminal("ISCRIPT_END"),
                TokenType::HtmlStart => Symbol::terminal("HTML_START"),
                TokenType::HtmlEnd => Symbol::terminal("HTML_END"),
                TokenType::BlockCommentStart => Symbol::terminal("BLOCK_COMMENT_START"),
                TokenType::BlockCommentEnd => Symbol::terminal("BLOCK_COMMENT_END"),
                TokenType::LineComment(_) => Symbol::terminal("LINE_COMMENT"),
                TokenType::Sharp => Symbol::terminal("SHARP"),
                TokenType::Asterisk => Symbol::terminal("ASTERISK"),
                TokenType::At => Symbol::terminal("AT"),
                TokenType::LBracket => Symbol::terminal("LBRACKET"),
                TokenType::RBracket => Symbol::terminal("RBRACKET"),
                TokenType::Underscore => Symbol::terminal("UNDERSCORE"),
                TokenType::Equal => Symbol::terminal("EQUAL"),
                TokenType::Colon => Symbol::terminal("COLON"),
                TokenType::Pipe => Symbol::terminal("PIPE"),
                TokenType::Newline => Symbol::terminal("NEWLINE"),
                TokenType::Entity(_) => Symbol::terminal("ENTITY"),
                TokenType::ParamRef(_) => Symbol::terminal("PARAM_REF"),
                TokenType::Eof => Symbol::terminal("$"),
            },
            None => Symbol::terminal("$"),
        }
    }

    fn create_terminal_node(&self, token: &Token) -> AstNode {
        AstNode::new_text(token.lexeme().into_owned(), false)
    }

    fn create_non_terminal_node(
        &self,
        production: &Production,
        children: Vec<Box<AstNode>>,
    ) -> AstNode {
        let Symbol::NonTerminal(ref name) = production.lhs else {
            return AstNode::new_scenario(children);
        };

        match name.as_ref() {
            "scenario" => match children.into_iter().next() {
                Some(node) => *node,
                None => AstNode::new_scenario(vec![]),
            },
            "line_list" | "mixed_content" => {
                // Flatten the left-recursive lists into a single Scenario,
                // dropping the NEWLINE separator terminals.
                let mut flattened = Vec::new();
                for child in children {
                    match *child {
                        AstNode::Scenario { lines } => flattened.extend(lines),
                        AstNode::Text { ref content, .. } if content == "\n" => {}
                        _ => flattened.push(child),
                    }
                }
                AstNode::new_scenario(flattened)
            }
            "line" | "text_segment" | "parameter_value" | "param_name" => {
                match children.into_iter().next() {
                    Some(node) => *node,
                    None => AstNode::new_scenario(vec![]),
                }
            }
            "text_line" => {
                let preserve = production.rhs.first() == Some(&Symbol::terminal("UNDERSCORE"));
                let mut iter = children.into_iter();
                if preserve {
                    iter.next(); // drop the "_" marker terminal
                }
                let mut flattened = Vec::new();
                for child in iter {
                    match *child {
                        AstNode::Scenario { lines } => {
                            for line in lines {
                                flattened.push(Box::new(mark_preserve(*line, preserve)));
                            }
                        }
                        node => flattened.push(Box::new(mark_preserve(node, preserve))),
                    }
                }
                AstNode::new_scenario(flattened)
            }
            "character_name" => match children.into_iter().nth(1) {
                Some(spec) => *spec,
                None => AstNode::new_character(String::new(), None),
            },
            "chara_spec" => {
                let (name, face) = split_spec(&production.rhs, &children, "COLON");
                AstNode::new_character(name, face)
            }
            "label" => match children.into_iter().nth(1) {
                Some(spec) => *spec,
                None => AstNode::new_label(String::new(), None),
            },
            "label_spec" => {
                let (name, text) = split_spec(&production.rhs, &children, "PIPE");
                AstNode::new_label(name, text)
            }
            "tag_at" => match children.into_iter().nth(1) {
                Some(tag) => match *tag {
                    AstNode::Tag {
                        name, parameters, ..
                    } => AstNode::new_tag(name, parameters, true),
                    other => other,
                },
                None => AstNode::new_scenario(vec![]),
            },
            "bracket_tag" => match children.into_iter().nth(1) {
                Some(tag) => *tag,
                None => AstNode::new_scenario(vec![]),
            },
            "tag_content" => {
                let mut iter = children.into_iter();
                let name = iter
                    .next()
                    .map(|n| n.flatten_text())
                    .unwrap_or_default();
                let mut parameters = Vec::new();
                if let Some(list) = iter.next()
                    && let AstNode::Scenario { lines } = *list
                {
                    for node in lines {
                        if let AstNode::TagParameter { name, value } = *node {
                            parameters.push(Parameter { name, value });
                        }
                    }
                }
                AstNode::new_tag(name, parameters, false)
            }
            "parameter_list" => {
                let mut params = Vec::new();
                for child in children {
                    match *child {
                        AstNode::Scenario { lines } => params.extend(lines),
                        AstNode::TagParameter { .. } => params.push(child),
                        // A bare ASTERISK terminal: macro parameter pass-through.
                        AstNode::Text { ref content, .. } if content == "*" => {
                            params.push(Box::new(AstNode::TagParameter {
                                name: "*".to_string(),
                                value: None,
                            }));
                        }
                        _ => {}
                    }
                }
                AstNode::new_scenario(params)
            }
            "parameter" => {
                let mut iter = children.into_iter();
                let name = iter
                    .next()
                    .map(|n| n.flatten_text())
                    .unwrap_or_default();
                let has_equal = iter.next().is_some();
                let value = iter.next().map(|n| n.flatten_text());
                AstNode::TagParameter {
                    name,
                    value: if has_equal {
                        Some(value.unwrap_or_default())
                    } else {
                        None
                    },
                }
            }
            "line_comment" => {
                let content = children
                    .first()
                    .map(|n| n.flatten_text())
                    .unwrap_or_default();
                AstNode::new_comment(content, false)
            }
            "block_comment_start" | "block_comment_end" => {
                let content = children
                    .first()
                    .map(|n| n.flatten_text())
                    .unwrap_or_default();
                AstNode::new_comment(content, true)
            }
            "iscript_block" => {
                let content = children
                    .get(1)
                    .map(|n| n.flatten_text())
                    .unwrap_or_default();
                AstNode::Script { content }
            }
            "html_block" => {
                let content = children
                    .get(1)
                    .map(|n| n.flatten_text())
                    .unwrap_or_default();
                AstNode::Html { content }
            }
            "script_content" | "html_content" => {
                let mut text = String::new();
                for child in children {
                    text.push_str(&child.flatten_text());
                }
                AstNode::new_text(text, false)
            }
            _ => AstNode::new_scenario(children),
        }
    }
}

fn mark_preserve(node: AstNode, preserve: bool) -> AstNode {
    match node {
        AstNode::Text { content, .. } if preserve => AstNode::new_text(content, true),
        other => other,
    }
}

/// Interpret a `chara_spec` / `label_spec` reduction. The lexer emits at most
/// `TEXT? SEPARATOR TEXT?`; which pieces are present is determined by the
/// production's RHS shape.
fn split_spec(
    rhs: &[Symbol],
    children: &[Box<AstNode>],
    separator: &str,
) -> (String, Option<String>) {
    let sep = Symbol::terminal(separator);
    let sep_pos = rhs.iter().position(|s| *s == sep);

    match sep_pos {
        // TEXT only: name without separator.
        None => (
            children
                .first()
                .map(|n| n.flatten_text())
                .unwrap_or_default(),
            None,
        ),
        Some(0) => {
            // Separator first: empty name; value present iff a TEXT follows.
            let value = children.get(1).map(|n| n.flatten_text());
            (String::new(), Some(value.unwrap_or_default()))
        }
        Some(_) => {
            let name = children
                .first()
                .map(|n| n.flatten_text())
                .unwrap_or_default();
            let value = children.get(2).map(|n| n.flatten_text());
            (name, Some(value.unwrap_or_default()))
        }
    }
}
