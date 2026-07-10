use clap::{Parser, Subcommand};
use std::path::PathBuf;

#[derive(Parser)]
#[command(name = "tyrano-parser-debugger")]
#[command(about = "Debug and visualize TyranoScript parser structures")]
pub struct Cli {
    #[command(subcommand)]
    pub command: Commands,
}

#[derive(Subcommand)]
pub enum Commands {
    /// Parse a .ks file and display CST in tree-sitter style format
    Tree {
        /// Path to the .ks file to parse
        #[arg(short, long)]
        file: PathBuf,
    },
    /// Generate Graphviz DOT files for parser visualization
    Graph {
        /// Output directory for DOT files
        #[arg(short, long, default_value = "debug_artifacts")]
        output: PathBuf,
        /// Only generate automaton (skip parse table)
        #[arg(long)]
        automaton_only: bool,
    },
    Parse {
        /// Path to the .ks file to parse
        #[arg(short, long)]
        file: PathBuf,
    },
}

