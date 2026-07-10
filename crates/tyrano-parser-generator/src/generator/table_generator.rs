use crate::Result;
use crate::grammar::{GrammarParser, Symbol};
use crate::state::{LRState, StateMachine};
use log::warn;
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, hash_map::Entry};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum Action {
    Shift(usize),
    Reduce(usize),
    Accept,
    Error,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct ParseTable {
    pub action_table: HashMap<(usize, Symbol), Action>,
    pub goto_table: HashMap<(usize, Symbol), usize>,
}

pub struct TableGenerator {
    state_machine: StateMachine,
    grammar: GrammarParser,
}

impl TableGenerator {
    pub fn new(grammar: GrammarParser) -> Result<Self> {
        let state_machine = StateMachine::new(grammar.clone())?;

        Ok(TableGenerator {
            state_machine,
            grammar,
        })
    }

    pub fn generate_parse_table(&self) -> Result<ParseTable> {
        let mut action_table = HashMap::new();
        let mut goto_table = HashMap::new();

        for state in self.state_machine.get_states() {
            self.fill_action_table(state, &mut action_table)?;
            self.fill_goto_table(state, &mut goto_table);
        }

        Ok(ParseTable {
            action_table,
            goto_table,
        })
    }

    fn fill_action_table(
        &self,
        state: &LRState,
        action_table: &mut HashMap<(usize, Symbol), Action>,
    ) -> Result<()> {
        for item in &state.items {
            if item.is_complete() {
                // Reduce action
                if item.production.lhs == *self.grammar.get_start_symbol()
                    && item.lookahead == Symbol::terminal("$")
                {
                    Self::insert_accept_action(action_table, state.id, item.lookahead.clone())?;
                } else {
                    Self::insert_reduce_action(
                        action_table,
                        state.id,
                        item.lookahead.clone(),
                        item.production.id,
                    )?;
                }
            } else if let Some(next_symbol) = item.next_symbol()
                && next_symbol.is_terminal()
            {
                // Shift action
                if let Some(&next_state) = state.transitions.get(next_symbol) {
                    Self::insert_shift_action(
                        action_table,
                        state.id,
                        next_symbol.clone(),
                        next_state,
                    )?;
                }
            }
        }

        Ok(())
    }

    fn insert_shift_action(
        action_table: &mut HashMap<(usize, Symbol), Action>,
        state_id: usize,
        symbol: Symbol,
        next_state: usize,
    ) -> Result<()> {
        match action_table.entry((state_id, symbol)) {
            Entry::Vacant(entry) => {
                entry.insert(Action::Shift(next_state));
            }
            Entry::Occupied(mut entry) => match entry.get() {
                Action::Shift(existing) => {
                    if *existing != next_state {
                        return Err(crate::ParserError::StateError(format!(
                            "Shift-shift conflict in state {state_id}",
                        )));
                    }
                }
                Action::Reduce(_) | Action::Error => {
                    // Prefer shift over reduce or explicit error slots
                    warn!("Shift-reduce conflict in state {state_id}, preferring shift action.");
                    entry.insert(Action::Shift(next_state));
                }
                Action::Accept => {
                    return Err(crate::ParserError::StateError(format!(
                        "Accept-shift conflict in state {state_id}",
                    )));
                }
            },
        }

        Ok(())
    }

    fn insert_reduce_action(
        action_table: &mut HashMap<(usize, Symbol), Action>,
        state_id: usize,
        lookahead: Symbol,
        production_id: usize,
    ) -> Result<()> {
        match action_table.entry((state_id, lookahead)) {
            Entry::Vacant(entry) => {
                entry.insert(Action::Reduce(production_id));
            }
            Entry::Occupied(mut entry) => match entry.get() {
                Action::Shift(_) => {
                    // Shift already recorded for this slot; prefer it over reduce
                }
                Action::Reduce(existing) => {
                    if *existing != production_id {
                        return Err(crate::ParserError::StateError(format!(
                            "Reduce-reduce conflict in state {state_id}",
                        )));
                    }
                }
                Action::Accept => {
                    // Accept takes precedence; keep existing
                }
                Action::Error => {
                    entry.insert(Action::Reduce(production_id));
                }
            },
        }

        Ok(())
    }

    fn insert_accept_action(
        action_table: &mut HashMap<(usize, Symbol), Action>,
        state_id: usize,
        lookahead: Symbol,
    ) -> Result<()> {
        match action_table.entry((state_id, lookahead)) {
            Entry::Vacant(entry) => {
                entry.insert(Action::Accept);
            }
            Entry::Occupied(mut entry) => match entry.get() {
                Action::Accept => {}
                Action::Reduce(_) | Action::Error => {
                    entry.insert(Action::Accept);
                }
                Action::Shift(_) => {
                    return Err(crate::ParserError::StateError(format!(
                        "Accept-shift conflict in state {state_id}",
                    )));
                }
            },
        }

        Ok(())
    }

    fn fill_goto_table(&self, state: &LRState, goto_table: &mut HashMap<(usize, Symbol), usize>) {
        for (symbol, &next_state) in &state.transitions {
            if symbol.is_non_terminal() {
                goto_table.insert((state.id, symbol.clone()), next_state);
            }
        }
    }

    pub fn get_parse_table(&self) -> Result<ParseTable> {
        self.generate_parse_table()
    }
}

#[cfg(test)]
mod tests {
    use super::{Action, TableGenerator};
    use crate::ParserError;
    use crate::grammar::Symbol;
    use std::collections::HashMap;

    #[test]
    fn shift_overwrites_prior_reduce() {
        let mut table = HashMap::new();
        let symbol = Symbol::terminal("tok");

        TableGenerator::insert_reduce_action(&mut table, 0, symbol.clone(), 1).unwrap();
        TableGenerator::insert_shift_action(&mut table, 0, symbol.clone(), 2).unwrap();

        let key = (0usize, symbol);
        assert!(matches!(table.get(&key), Some(Action::Shift(2))));
    }

    #[test]
    fn reduce_does_not_override_existing_shift() {
        let mut table = HashMap::new();
        let symbol = Symbol::terminal("tok");

        TableGenerator::insert_shift_action(&mut table, 1, symbol.clone(), 3).unwrap();
        TableGenerator::insert_reduce_action(&mut table, 1, symbol.clone(), 2).unwrap();

        let key = (1usize, symbol);
        assert!(matches!(table.get(&key), Some(Action::Shift(3))));
    }

    #[test]
    fn reduce_reduce_conflict_is_reported() {
        let mut table = HashMap::new();
        let symbol = Symbol::terminal("tok");

        TableGenerator::insert_reduce_action(&mut table, 2, symbol.clone(), 1).unwrap();
        let err =
            TableGenerator::insert_reduce_action(&mut table, 2, symbol.clone(), 99).unwrap_err();

        assert!(matches!(err, ParserError::StateError(_)));
    }

    #[test]
    fn shift_shift_conflict_is_reported() {
        let mut table = HashMap::new();
        let symbol = Symbol::terminal("tok");

        TableGenerator::insert_shift_action(&mut table, 3, symbol.clone(), 4).unwrap();
        let err =
            TableGenerator::insert_shift_action(&mut table, 3, symbol.clone(), 5).unwrap_err();

        assert!(matches!(err, ParserError::StateError(_)));
    }

    #[test]
    fn accept_overrides_prior_reduce() {
        let mut table = HashMap::new();
        let symbol = Symbol::terminal("$");

        TableGenerator::insert_reduce_action(&mut table, 4, symbol.clone(), 7).unwrap();
        TableGenerator::insert_accept_action(&mut table, 4, symbol.clone()).unwrap();

        assert!(matches!(
            table.get(&(4, symbol.clone())),
            Some(Action::Accept)
        ));
    }

    #[test]
    fn accept_is_stable_against_later_reduce() {
        let mut table = HashMap::new();
        let symbol = Symbol::terminal("$");

        TableGenerator::insert_accept_action(&mut table, 5, symbol.clone()).unwrap();
        TableGenerator::insert_reduce_action(&mut table, 5, symbol.clone(), 9).unwrap();

        assert!(matches!(table.get(&(5, symbol)), Some(Action::Accept)));
    }
}
