use serde::{Deserialize, Serialize};
use std::{fmt, sync::Arc};

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub enum Symbol {
    Terminal(Arc<str>),
    NonTerminal(Arc<str>),
    Epsilon,
}

impl Symbol {
    pub fn terminal(value: impl Into<String>) -> Self {
        Symbol::Terminal(Arc::<str>::from(value.into()))
    }

    pub fn non_terminal(value: impl Into<String>) -> Self {
        Symbol::NonTerminal(Arc::<str>::from(value.into()))
    }

    pub fn as_str(&self) -> Option<&str> {
        match self {
            Symbol::Terminal(s) | Symbol::NonTerminal(s) => Some(s.as_ref()),
            Symbol::Epsilon => None,
        }
    }

    pub fn is_terminal(&self) -> bool {
        matches!(self, Symbol::Terminal(_))
    }

    pub fn is_non_terminal(&self) -> bool {
        matches!(self, Symbol::NonTerminal(_))
    }

    pub fn is_epsilon(&self) -> bool {
        matches!(self, Symbol::Epsilon)
    }

    /// Get the name of this symbol as a string
    pub fn name(&self) -> &str {
        match self {
            Symbol::Terminal(s) | Symbol::NonTerminal(s) => s.as_ref(),
            Symbol::Epsilon => "ε",
        }
    }
}

impl fmt::Display for Symbol {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Symbol::Terminal(s) => write!(f, "'{s}'"),
            Symbol::NonTerminal(s) => write!(f, "{s}"),
            Symbol::Epsilon => write!(f, "ε"),
        }
    }
}
