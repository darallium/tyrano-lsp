use clap::{Parser, Subcommand};
use std::path::PathBuf;

#[derive(Parser)]
#[command(name = "tyrano-parser-generator")]
#[command(about = "LALR(1) Parser Generator for TyranoScript", long_about = None)]
pub struct Cli {
    #[command(subcommand)]
    pub command: Option<Commands>,
}

#[derive(Subcommand)]
pub enum Commands {
    /// Generate parser from grammar file
    Build {
        /// Grammar file path
        #[arg(short, long, default_value = "grammar/tyranoscript.grammar")]
        grammar: PathBuf,

        /// Output directory for generated parser crate
        #[arg(short, long, default_value = "crates/tyrano-parser")]
        output: PathBuf,
    },

    /// Check grammar for conflicts and issues without generating
    Check {
        /// Grammar file path
        #[arg(short, long, default_value = "grammar/tyranoscript.grammar")]
        grammar: PathBuf,
    },

    /// Generate debug artifacts (automaton.dot, conflicts.txt, etc.)
    Debug {
        /// Grammar file path
        #[arg(short, long, default_value = "grammar/tyranoscript.grammar")]
        grammar: PathBuf,

        /// Output directory for debug artifacts
        #[arg(short, long, default_value = "debug_artifacts")]
        output: PathBuf,
    },

    /// Generate Graphviz visualization of parser tables and automaton
    Graph {
        /// Grammar file path
        #[arg(short, long, default_value = "grammar/tyranoscript.grammar")]
        grammar: PathBuf,

        /// Output directory for generated DOT files
        #[arg(short, long, default_value = "debug_artifacts")]
        output: PathBuf,

        /// Generate only the automaton (skip table output)
        #[arg(long, default_value = "false")]
        automaton_only: bool,
    },
}
