use std::fs;
use std::path::Path;
use std::collections::HashMap;

use tyrano_parser_generator::{
    generator::TableGenerator,
    grammar::GrammarParser,
    state::StateMachine,
    visualizer,
};

pub fn run_graph(output_dir: &str, automaton_only: bool) -> Result<(), String> {
    let grammar_path = Path::new("grammar/tyranoscript.grammar");

    println!("Generating Graphviz visualization...");

    fs::create_dir_all(output_dir)
        .map_err(|e| format!("Failed to create output directory: {}", e))?;

    let grammar_content = fs::read_to_string(grammar_path)
        .map_err(|e| format!("Failed to read grammar file: {}", e))?;

    let grammar = GrammarParser::from_content(&grammar_content)
        .map_err(|e| format!("Failed to parse grammar: {:?}", e))?;

    // Build state machine (for automaton visualization)
    println!("Building LR state machine...");
    let state_machine = StateMachine::new(grammar.clone())
        .map_err(|e| format!("Failed to build state machine: {:?}", e))?;
    let states = state_machine.get_states();

    // Generate automaton DOT file
    println!("Generating automaton.dot...");
    let automaton_dot = visualizer::generate_automaton_dot(states, &grammar);
    let automaton_path = Path::new(output_dir).join("automaton.dot");
    fs::write(&automaton_path, automaton_dot)
        .map_err(|e| format!("Failed to write automaton.dot: {}", e))?;
    println!("  Written: {:?}", automaton_path);

    if !automaton_only {
        // Build parse table (for table visualization)
        println!("Building parse tables...");
        let table_generator = TableGenerator::new(grammar.clone())
            .map_err(|e| format!("Failed to create table generator: {:?}", e))?;
        let parse_table = table_generator
            .generate_parse_table()
            .map_err(|e| format!("Failed to generate parse table: {:?}", e))?;

        // Build symbol to ID mapping
        let mut symbol_to_id: HashMap<String, u32> = HashMap::new();
        let mut id = 0u32;
        for terminal in grammar.get_terminals() {
            symbol_to_id.insert(terminal.name().to_string(), id);
            id += 1;
        }
        symbol_to_id.insert("$".to_string(), id);
        id += 1;
        for non_terminal in grammar.get_non_terminals() {
            symbol_to_id.insert(non_terminal.name().to_string(), id);
            id += 1;
        }

        // Generate table DOT file
        println!("Generating table.dot...");
        let table_dot = visualizer::generate_table_dot(&parse_table, &grammar, &symbol_to_id);
        let table_path = Path::new(output_dir).join("table.dot");
        fs::write(&table_path, table_dot)
            .map_err(|e| format!("Failed to write table.dot: {}", e))?;
        println!("  Written: {:?}", table_path);
    }

    println!("\nGraphviz files generated successfully!");
    println!("To view the automaton:");
    println!(
        "  - Open https://dreampuf.github.io/GraphvizOnline/ and paste automaton.dot contents"
    );
    println!(
        "  - Or run: dot -Tsvg {:?} -o automaton.svg",
        automaton_path
    );

    Ok(())
}
