use super::lr_state::{LRItem, LRState};
use crate::Result;
use crate::grammar::{GrammarParser, Symbol};
use std::collections::{BTreeMap, HashMap, HashSet, VecDeque};
use std::rc::Rc;

pub struct StateMachine {
    states: Vec<LRState>,
    grammar: GrammarParser,
    first_sets: HashMap<Symbol, HashSet<Symbol>>,
    follow_sets: HashMap<Symbol, HashSet<Symbol>>,
}

impl StateMachine {
    pub fn new(grammar: GrammarParser) -> Result<Self> {
        let mut machine = StateMachine {
            states: Vec::new(),
            grammar,
            first_sets: HashMap::new(),
            follow_sets: HashMap::new(),
        };

        machine.compute_first_sets();
        machine.compute_follow_sets();
        machine.build_states()?;

        Ok(machine)
    }

    fn compute_first_sets(&mut self) {
        // Initialize FIRST sets for terminals
        for terminal in self.grammar.get_terminals() {
            let mut set = HashSet::new();
            set.insert(terminal.clone());
            self.first_sets.insert(terminal.clone(), set);
        }

        // Initialize empty FIRST sets for non-terminals
        for non_terminal in self.grammar.get_non_terminals() {
            self.first_sets.insert(non_terminal.clone(), HashSet::new());
        }

        // Fixed-point computation
        let mut changed = true;
        while changed {
            changed = false;

            for production in self.grammar.get_productions() {
                let lhs = &production.lhs;
                let mut first_of_rhs = HashSet::new();

                if production.rhs.is_empty() || production.rhs[0].is_epsilon() {
                    first_of_rhs.insert(Symbol::Epsilon);
                } else {
                    for symbol in &production.rhs {
                        if let Some(first) = self.first_sets.get(symbol) {
                            let without_epsilon: HashSet<_> =
                                first.iter().filter(|s| !s.is_epsilon()).cloned().collect();
                            first_of_rhs.extend(without_epsilon);

                            if !first.contains(&Symbol::Epsilon) {
                                break;
                            }
                        }
                    }
                }

                if let Some(first) = self.first_sets.get_mut(lhs) {
                    let old_size = first.len();
                    first.extend(first_of_rhs);
                    if first.len() > old_size {
                        changed = true;
                    }
                }
            }
        }
    }

    fn compute_follow_sets(&mut self) {
        // Initialize FOLLOW sets
        for non_terminal in self.grammar.get_non_terminals() {
            self.follow_sets
                .insert(non_terminal.clone(), HashSet::new());
        }

        // Add $ to FOLLOW(start_symbol)
        if let Some(follow) = self.follow_sets.get_mut(self.grammar.get_start_symbol()) {
            follow.insert(Symbol::terminal("$"));
        }

        // Fixed-point computation
        let mut changed = true;
        while changed {
            changed = false;

            for production in self.grammar.get_productions() {
                for (i, symbol) in production.rhs.iter().enumerate() {
                    if symbol.is_non_terminal() {
                        let mut follow_to_add = HashSet::new();

                        // Check symbols after this one
                        let mut all_have_epsilon = true;
                        for j in (i + 1)..production.rhs.len() {
                            if let Some(first) = self.first_sets.get(&production.rhs[j]) {
                                let without_epsilon: HashSet<_> =
                                    first.iter().filter(|s| !s.is_epsilon()).cloned().collect();
                                follow_to_add.extend(without_epsilon);

                                if !first.contains(&Symbol::Epsilon) {
                                    all_have_epsilon = false;
                                    break;
                                }
                            }
                        }

                        // If all following symbols can be epsilon, add FOLLOW(lhs)
                        if all_have_epsilon
                            && let Some(follow_lhs) = self.follow_sets.get(&production.lhs)
                        {
                            follow_to_add.extend(follow_lhs.clone());
                        }

                        if let Some(follow) = self.follow_sets.get_mut(symbol) {
                            let old_size = follow.len();
                            follow.extend(follow_to_add);
                            if follow.len() > old_size {
                                changed = true;
                            }
                        }
                    }
                }
            }
        }
    }

    fn build_states(&mut self) -> Result<()> {
        // Create initial state
        let start_production = self
            .grammar
            .get_productions()
            .iter()
            .find(|p| p.lhs == *self.grammar.get_start_symbol())
            .cloned()
            .ok_or_else(|| {
                crate::ParserError::StateError("No start production found".to_string())
            })?;

        let initial_item = LRItem::new(start_production, 0, Symbol::terminal("$"));

        let mut initial_state = LRState::new(0);
        initial_state.add_item(initial_item);
        self.closure(&mut initial_state);

        #[cfg(debug_assertions)]
        {
            eprintln!("=== Initial State (State 0) ===");
            eprintln!("Number of items: {}", initial_state.items.len());
            for item in &initial_state.items {
                eprintln!("  {item:?}");
            }
        }

        self.states.push(initial_state);

        // Build remaining states
        let mut queue = VecDeque::new();
        queue.push_back(0);
        let mut processed = HashSet::new();

        while let Some(state_id) = queue.pop_front() {
            if processed.contains(&state_id) {
                continue;
            }
            processed.insert(state_id);

            let transitions = self.compute_transitions(state_id)?;

            for (symbol, new_state) in transitions {
                let new_state_id = self.add_or_get_state(new_state);
                self.states[state_id].add_transition(symbol, new_state_id);

                if !processed.contains(&new_state_id) {
                    queue.push_back(new_state_id);
                }
            }
        }

        Ok(())
    }

    fn closure(&self, state: &mut LRState) {
        let mut changed = true;

        while changed {
            changed = false;
            let mut new_items = Vec::new();

            for item in &state.items {
                if let Some(next_symbol) = item.next_symbol()
                    && next_symbol.is_non_terminal()
                {
                    for production in self.grammar.get_productions() {
                        if production.lhs == *next_symbol {
                            let lookaheads = self.compute_lookaheads(item);

                            for lookahead in lookaheads {
                                let new_item = LRItem::new(Rc::clone(production), 0, lookahead);

                                if !state.items.contains(&new_item) {
                                    new_items.push(new_item);
                                    changed = true;
                                }
                            }
                        }
                    }
                }
            }

            for item in new_items {
                state.add_item(item);
            }
        }
    }

    fn compute_lookaheads(&self, item: &LRItem) -> HashSet<Symbol> {
        let mut lookaheads = HashSet::new();

        // Get symbols after the non-terminal
        let mut beta = Vec::new();
        for i in (item.dot_position + 1)..item.production.rhs.len() {
            beta.push(&item.production.rhs[i]);
        }
        beta.push(&item.lookahead);

        // Compute FIRST(beta)
        lookaheads.extend(self.first_of_sequence(&beta));

        lookaheads
    }

    fn first_of_sequence(&self, symbols: &[&Symbol]) -> HashSet<Symbol> {
        let mut result = HashSet::new();

        for symbol in symbols {
            if let Some(first) = self.first_sets.get(*symbol) {
                let without_epsilon: HashSet<_> =
                    first.iter().filter(|s| !s.is_epsilon()).cloned().collect();
                result.extend(without_epsilon);

                if !first.contains(&Symbol::Epsilon) {
                    break;
                }
            } else {
                result.insert((*symbol).clone());
                break;
            }
        }

        result
    }

    fn compute_transitions(&self, state_id: usize) -> Result<BTreeMap<Symbol, LRState>> {
        let state = &self.states[state_id];
        let mut transitions = BTreeMap::new();

        // Group items by their next symbol
        let mut grouped: BTreeMap<Symbol, Vec<LRItem>> = BTreeMap::new();

        for item in &state.items {
            if let Some(next_symbol) = item.next_symbol() {
                grouped
                    .entry(next_symbol.clone())
                    .or_default()
                    .push(item.advance());
            }
        }

        // Create new states for each transition
        for (symbol, items) in grouped {
            let mut new_state = LRState::new(self.states.len());
            for item in items {
                new_state.add_item(item);
            }
            self.closure(&mut new_state);
            transitions.insert(symbol, new_state);
        }

        Ok(transitions)
    }

    fn add_or_get_state(&mut self, state: LRState) -> usize {
        // Check if state already exists
        for existing_state in &self.states {
            if existing_state.items == state.items {
                return existing_state.id;
            }
        }

        // Add new state
        let id = self.states.len();
        let mut new_state = state;
        new_state.id = id;
        self.states.push(new_state);
        id
    }

    pub fn get_states(&self) -> &[LRState] {
        &self.states
    }
}
