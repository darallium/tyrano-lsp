use clap::Parser;
use std::fs;
use std::path::Path;
use tyrano_parser_generator::{
    ParserError, ParserGeneratorCodegen, Result,
    cli::{Cli, Commands},
    generator::TableGenerator,
    grammar::GrammarParser,
    visualizer,
};

fn main() -> Result<()> {
    let cli = Cli::parse();

    match cli.command {
        Some(Commands::Build { grammar, output }) => {
            build_parser(&grammar, &output)?;
        }
        Some(Commands::Check { grammar }) => {
            check_grammar(&grammar)?;
        }
        Some(Commands::Debug { grammar, output }) => {
            generate_debug_artifacts(&grammar, &output)?;
        }
        Some(Commands::Graph {
            grammar,
            output,
            automaton_only,
        }) => {
            generate_graph(&grammar, &output, automaton_only)?;
        }
        None => {
            // Default: build with default paths
            let grammar = Path::new("grammar/tyranoscript.grammar");
            let output = Path::new("crates/tyrano-parser");
            build_parser(grammar, output)?;
        }
    }

    Ok(())
}

/// Build parser from grammar file
fn build_parser(grammar_path: &Path, output_dir: &Path) -> Result<()> {
    println!("Reading grammar from: {:?}", grammar_path);

    // Read grammar file
    let grammar_content = fs::read_to_string(grammar_path)
        .map_err(|e| ParserError::GrammarError(format!("Failed to read grammar file: {}", e)))?;

    // Parse grammar
    let grammar = parse_grammar_content(&grammar_content)?;

    // Build parse tables
    println!("Building LALR(1) parse tables...");
    let table_generator = TableGenerator::new(grammar.clone())?;
    let parse_table = table_generator.generate_parse_table()?;

    println!(
        "Generated {} action entries, {} goto entries",
        parse_table.action_table.len(),
        parse_table.goto_table.len()
    );

    // Generate Rust code
    println!("Generating parser to: {:?}", output_dir);
    let codegen = ParserGeneratorCodegen::new(output_dir, &grammar, &parse_table);
    codegen.generate()?;

    println!("Parser generation complete!");
    println!("Run 'cargo check -p tyrano-parser' to verify the generated code.");

    Ok(())
}

/// Check grammar for conflicts without generating
fn check_grammar(grammar_path: &Path) -> Result<()> {
    println!("Checking grammar: {:?}", grammar_path);

    let grammar_content = fs::read_to_string(grammar_path)
        .map_err(|e| ParserError::GrammarError(format!("Failed to read grammar file: {}", e)))?;

    let grammar = parse_grammar_content(&grammar_content)?;

    // Build tables to check for conflicts
    let table_generator = TableGenerator::new(grammar)?;
    let parse_table = table_generator.generate_parse_table()?;

    println!("Grammar check passed!");
    println!("  {} productions", parse_table.action_table.len());
    println!("  {} goto entries", parse_table.goto_table.len());
    println!("No conflicts detected.");

    Ok(())
}

/// Generate debug artifacts (automaton.dot, conflicts.txt, etc.)
fn generate_debug_artifacts(grammar_path: &Path, output_dir: &Path) -> Result<()> {
    println!("Generating debug artifacts for: {:?}", grammar_path);

    fs::create_dir_all(output_dir).map_err(|e| {
        ParserError::GrammarError(format!("Failed to create output directory: {}", e))
    })?;

    let grammar_content = fs::read_to_string(grammar_path)
        .map_err(|e| ParserError::GrammarError(format!("Failed to read grammar file: {}", e)))?;

    let grammar = parse_grammar_content(&grammar_content)?;

    // Output FIRST/FOLLOW sets
    let first_follow_path = output_dir.join("first_follow.txt");
    let mut ff_content = String::new();
    ff_content.push_str("=== FIRST Sets ===\n");
    for terminal in grammar.get_terminals() {
        ff_content.push_str(&format!(
            "FIRST({}) = {{{}}}\n",
            terminal.name(),
            terminal.name()
        ));
    }
    ff_content.push_str("\n=== Non-terminals ===\n");
    for nt in grammar.get_non_terminals() {
        ff_content.push_str(&format!("  {}\n", nt.name()));
    }

    fs::write(&first_follow_path, ff_content).map_err(|e| {
        ParserError::GrammarError(format!("Failed to write first_follow.txt: {}", e))
    })?;

    // Output productions
    let productions_path = output_dir.join("productions.txt");
    let mut prod_content = String::new();
    prod_content.push_str("=== Productions ===\n");
    for prod in grammar.get_productions() {
        let rhs: Vec<String> = prod.rhs.iter().map(|s| s.name().to_string()).collect();
        prod_content.push_str(&format!(
            "{}: {} -> {}\n",
            prod.id,
            prod.lhs.name(),
            rhs.join(" ")
        ));
    }

    fs::write(&productions_path, prod_content).map_err(|e| {
        ParserError::GrammarError(format!("Failed to write productions.txt: {}", e))
    })?;

    println!("Debug artifacts written to: {:?}", output_dir);

    Ok(())
}

/// Parse grammar content into GrammarParser
fn parse_grammar_content(content: &str) -> Result<GrammarParser> {
    GrammarParser::from_content(content)
}

/// Generate Graphviz visualization files for parser tables and automaton
fn generate_graph(grammar_path: &Path, output_dir: &Path, automaton_only: bool) -> Result<()> {
    use std::collections::HashMap;
    use tyrano_parser_generator::state::StateMachine;

    println!("Generating Graphviz visualization for: {:?}", grammar_path);

    fs::create_dir_all(output_dir).map_err(|e| {
        ParserError::GrammarError(format!("Failed to create output directory: {}", e))
    })?;

    let grammar_content = fs::read_to_string(grammar_path)
        .map_err(|e| ParserError::GrammarError(format!("Failed to read grammar file: {}", e)))?;

    let grammar = parse_grammar_content(&grammar_content)?;

    // Build state machine (for automaton visualization)
    println!("Building LR state machine...");
    let state_machine = StateMachine::new(grammar.clone())?;
    let states = state_machine.get_states();

    // Generate automaton DOT file
    println!("Generating automaton.dot...");
    let automaton_dot = visualizer::generate_automaton_dot(states, &grammar);
    let automaton_path = output_dir.join("automaton.dot");
    fs::write(&automaton_path, automaton_dot)
        .map_err(|e| ParserError::GrammarError(format!("Failed to write automaton.dot: {}", e)))?;
    println!("  Written: {:?}", automaton_path);

    if !automaton_only {
        // Build parse table (for table visualization)
        println!("Building parse tables...");
        let table_generator = TableGenerator::new(grammar.clone())?;
        let parse_table = table_generator.generate_parse_table()?;

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
        let table_path = output_dir.join("table.dot");
        fs::write(&table_path, table_dot)
            .map_err(|e| ParserError::GrammarError(format!("Failed to write table.dot: {}", e)))?;
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
