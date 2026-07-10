use super::{production::Production, symbol::Symbol};
use crate::Result;
use std::collections::HashSet;
use std::rc::Rc;

#[derive(Clone)]
pub struct GrammarParser {
    productions: Vec<Rc<Production>>,
    terminals: HashSet<Symbol>,
    non_terminals: HashSet<Symbol>,
    start_symbol: Symbol,
}

impl GrammarParser {
    /// Create a new GrammarParser from grammar text content
    pub fn from_content(grammar_text: &str) -> Result<Self> {
        let mut parser = GrammarParser {
            productions: Vec::new(),
            terminals: HashSet::new(),
            non_terminals: HashSet::new(),
            start_symbol: Symbol::non_terminal("scenario"),
        };

        parser.parse_grammar(grammar_text)?;
        Ok(parser)
    }

    fn parse_grammar(&mut self, grammar_text: &str) -> Result<()> {
        let lines: Vec<&str> = grammar_text.lines().collect();
        let mut in_rules_section = false;
        let mut current_lhs: Option<Symbol> = None;
        let mut production_id = 0;

        for line in lines {
            let line = line.trim();

            if line.is_empty() || line.starts_with("//") {
                continue;
            }

            if line == "%%" {
                in_rules_section = !in_rules_section;
                continue;
            }

            if !in_rules_section {
                if line.starts_with("%start") {
                    let parts: Vec<&str> = line.split_whitespace().collect();
                    if parts.len() >= 2 {
                        self.start_symbol = Symbol::non_terminal(parts[1]);
                    }
                } else if line.starts_with("%token") {
                    let tokens = line[6..].trim();
                    for token in tokens.split_whitespace() {
                        self.terminals.insert(Symbol::terminal(token));
                    }
                }
            } else {
                // Parse production rules
                if line.ends_with(':') {
                    // Handle "rule_name:" format
                    let lhs = line[..line.len() - 1].trim();
                    current_lhs = Some(Symbol::non_terminal(lhs));
                    self.non_terminals.insert(Symbol::non_terminal(lhs));
                } else if line.starts_with('|') || line.starts_with(':') {
                    // Handle continuation lines that start with | or :
                    if line.starts_with(':') && current_lhs.is_none() {
                        continue;
                    }

                    if let Some(ref lhs) = current_lhs {
                        let rhs_str = if line.starts_with('|') {
                            &line[1..]
                        } else {
                            &line[1..]
                        }
                        .trim();

                        let rhs = self.parse_rhs(rhs_str)?;
                        let production = Rc::new(Production::new(production_id, lhs.clone(), rhs));
                        self.productions.push(production);
                        production_id += 1;
                    }
                } else if line == ";" {
                    current_lhs = None;
                } else {
                    // Handle bare non-terminal name (LHS without colon)
                    if !line.contains(':') && !line.contains('|') {
                        current_lhs = Some(Symbol::non_terminal(line));
                        self.non_terminals.insert(Symbol::non_terminal(line));
                    }
                }
            }
        }

        Ok(())
    }

    fn parse_rhs(&self, rhs_str: &str) -> Result<Vec<Symbol>> {
        if rhs_str == "/* empty */" {
            return Ok(vec![Symbol::Epsilon]);
        }

        let mut symbols = Vec::new();
        let tokens: Vec<&str> = rhs_str.split_whitespace().collect();

        for token in tokens {
            let symbol = if self.is_terminal(token) {
                Symbol::terminal(token)
            } else {
                Symbol::non_terminal(token)
            };
            symbols.push(symbol);
        }

        Ok(symbols)
    }

    fn is_terminal(&self, token: &str) -> bool {
        token.chars().all(|c| c.is_uppercase() || c == '_')
    }

    pub fn get_productions(&self) -> &[Rc<Production>] {
        &self.productions
    }

    pub fn get_start_symbol(&self) -> &Symbol {
        &self.start_symbol
    }

    /// Terminals sorted by name so symbol IDs and generated code are deterministic
    pub fn get_terminals(&self) -> Vec<&Symbol> {
        let mut terminals: Vec<&Symbol> = self.terminals.iter().collect();
        terminals.sort_by(|a, b| a.name().cmp(b.name()));
        terminals
    }

    /// Non-terminals sorted by name so symbol IDs and generated code are deterministic
    pub fn get_non_terminals(&self) -> Vec<&Symbol> {
        let mut non_terminals: Vec<&Symbol> = self.non_terminals.iter().collect();
        non_terminals.sort_by(|a, b| a.name().cmp(b.name()));
        non_terminals
    }
}
