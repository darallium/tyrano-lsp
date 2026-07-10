use super::symbol::Symbol;

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Production {
    pub id: usize,
    pub lhs: Symbol,
    pub rhs: Vec<Symbol>,
    pub action: Option<String>,
}

impl Production {
    pub fn new(id: usize, lhs: Symbol, rhs: Vec<Symbol>) -> Self {
        Production {
            id,
            lhs,
            rhs,
            action: None,
        }
    }

    pub fn with_action(mut self, action: String) -> Self {
        self.action = Some(action);
        self
    }

    pub fn is_epsilon_production(&self) -> bool {
        self.rhs.len() == 1 && self.rhs[0].is_epsilon()
    }
}
