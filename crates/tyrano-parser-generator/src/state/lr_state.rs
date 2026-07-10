use crate::grammar::{Production, Symbol};
use std::collections::{BTreeMap, BTreeSet};
use std::fmt;
use std::rc::Rc;

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct LRItem {
    pub production: Rc<Production>,
    pub dot_position: usize,
    pub lookahead: Symbol,
}

impl LRItem {
    pub fn new(production: Rc<Production>, dot_position: usize, lookahead: Symbol) -> Self {
        LRItem {
            production,
            dot_position,
            lookahead,
        }
    }

    pub fn is_complete(&self) -> bool {
        // Epsilon productions are complete at dot_position 0
        if self.production.is_epsilon_production() {
            return self.dot_position == 0;
        }
        self.dot_position >= self.production.rhs.len()
    }

    pub fn next_symbol(&self) -> Option<&Symbol> {
        if self.is_complete() {
            None
        } else {
            Some(&self.production.rhs[self.dot_position])
        }
    }

    pub fn advance(&self) -> Self {
        LRItem {
            production: Rc::clone(&self.production),
            dot_position: self.dot_position + 1,
            lookahead: self.lookahead.clone(),
        }
    }
}

impl fmt::Display for LRItem {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{} ->", self.production.lhs)?;

        for (i, symbol) in self.production.rhs.iter().enumerate() {
            if i == self.dot_position {
                write!(f, " •")?;
            }
            write!(f, " {symbol}")?;
        }

        if self.dot_position >= self.production.rhs.len() {
            write!(f, " •")?;
        }

        write!(f, ", {}", self.lookahead)?;
        Ok(())
    }
}

#[derive(Debug, Clone)]
pub struct LRState {
    pub id: usize,
    pub items: BTreeSet<LRItem>,
    pub transitions: BTreeMap<Symbol, usize>,
}

impl LRState {
    pub fn new(id: usize) -> Self {
        LRState {
            id,
            items: BTreeSet::new(),
            transitions: BTreeMap::new(),
        }
    }

    pub fn add_item(&mut self, item: LRItem) {
        self.items.insert(item);
    }

    pub fn add_transition(&mut self, symbol: Symbol, state_id: usize) {
        self.transitions.insert(symbol, state_id);
    }
}
