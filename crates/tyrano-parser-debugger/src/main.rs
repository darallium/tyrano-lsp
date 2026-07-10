use clap::Parser;
use tyrano_parser_debugger::{
    cli::{Cli, Commands},
    graph::run_graph,
    tree::run_tree,
    parser::run_parse,
};


fn main() {
    let cli = Cli::parse();

    match cli.command {
        Commands::Tree { file } => {
            let file = file.to_str().expect("Invalid file path");
            if let Err(e) = run_tree(file) {
                eprintln!("Error: {}", e);
                std::process::exit(1);
            }
        }
        Commands::Graph {
            output,
            automaton_only,
        } => {
            let output = output.to_str().expect("Invalid output path");
            if let Err(e) = run_graph(output, automaton_only) {
                eprintln!("Error: {}", e);
                std::process::exit(1);
            }
        }
        Commands::Parse { file } => {
            let file = file.to_str().expect("Invalid file path");
            if let Err(e) = run_parse(file) {
                eprintln!("Error: {}", e);
                std::process::exit(1);
            }
        }
    }
}



