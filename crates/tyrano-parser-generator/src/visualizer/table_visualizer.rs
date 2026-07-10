//! Parse table visualization using Graphviz DOT format

use crate::generator::{Action, ParseTable};
use crate::grammar::GrammarParser;
use std::collections::{BTreeSet, HashMap};

/// Generate a Graphviz DOT representation of the parse table
pub fn generate_table_dot(
    parse_table: &ParseTable,
    grammar: &GrammarParser,
    symbol_to_id: &HashMap<String, u32>,
) -> String {
    let mut dot = String::new();

    dot.push_str("digraph ParseTable {\n");
    dot.push_str("    rankdir=TB;\n");
    dot.push_str("    node [shape=plaintext];\n");
    dot.push_str("\n");

    // Collect all states
    let mut states: BTreeSet<usize> = BTreeSet::new();
    for (state, _) in parse_table.action_table.keys() {
        states.insert(*state);
    }
    for (state, _) in parse_table.goto_table.keys() {
        states.insert(*state);
    }

    // Get terminals and non-terminals
    let terminals: Vec<_> = grammar
        .get_terminals()
        .iter()
        .map(|t| t.name().to_string())
        .collect();
    let non_terminals: Vec<_> = grammar
        .get_non_terminals()
        .iter()
        .map(|nt| nt.name().to_string())
        .collect();

    // Build table as HTML-like label
    dot.push_str("    table [label=<\n");
    dot.push_str("        <TABLE BORDER=\"0\" CELLBORDER=\"1\" CELLSPACING=\"0\">\n");

    // Header row
    dot.push_str("        <TR>\n");
    dot.push_str("            <TD BGCOLOR=\"lightgray\"><B>State</B></TD>\n");
    for terminal in &terminals {
        dot.push_str(&format!(
            "            <TD BGCOLOR=\"#E6F3FF\"><B>{}</B></TD>\n",
            escape_html(terminal)
        ));
    }
    dot.push_str("            <TD BGCOLOR=\"#E6F3FF\"><B>$</B></TD>\n");
    for nt in &non_terminals {
        dot.push_str(&format!(
            "            <TD BGCOLOR=\"#E6FFE6\"><B>{}</B></TD>\n",
            escape_html(nt)
        ));
    }
    dot.push_str("        </TR>\n");

    // Data rows
    for state in &states {
        dot.push_str("        <TR>\n");
        dot.push_str(&format!(
            "            <TD BGCOLOR=\"lightgray\"><B>{}</B></TD>\n",
            state
        ));

        // Action table entries for terminals
        for terminal in &terminals {
            if let Some(&sym_id) = symbol_to_id.get(terminal) {
                let cell = format_action_cell(parse_table, *state, sym_id);
                dot.push_str(&format!("            <TD>{}</TD>\n", cell));
            } else {
                dot.push_str("            <TD></TD>\n");
            }
        }

        // EOF ($) entry
        if let Some(&sym_id) = symbol_to_id.get("$") {
            let cell = format_action_cell(parse_table, *state, sym_id);
            dot.push_str(&format!("            <TD>{}</TD>\n", cell));
        } else {
            dot.push_str("            <TD></TD>\n");
        }

        // Goto table entries for non-terminals
        for nt in &non_terminals {
            if let Some(&sym_id) = symbol_to_id.get(nt) {
                let cell = format_goto_cell(parse_table, *state, sym_id);
                dot.push_str(&format!("            <TD>{}</TD>\n", cell));
            } else {
                dot.push_str("            <TD></TD>\n");
            }
        }

        dot.push_str("        </TR>\n");
    }

    dot.push_str("        </TABLE>\n");
    dot.push_str("    >];\n");
    dot.push_str("}\n");

    dot
}

/// Format an action table cell
fn format_action_cell(parse_table: &ParseTable, state: usize, sym_id: u32) -> String {
    // Find action by iterating over action_table
    for ((s, symbol), action) in &parse_table.action_table {
        if *s == state {
            // Need to check if symbol matches sym_id - for now, we check by iteration
            // This is a workaround since we don't have direct symbol->id lookup in ParseTable
            if symbol_matches_id(symbol, sym_id) {
                return match action {
                    Action::Shift(next) => format!("<FONT COLOR=\"blue\">s{}</FONT>", next),
                    Action::Reduce(prod) => format!("<FONT COLOR=\"red\">r{}</FONT>", prod),
                    Action::Accept => "<FONT COLOR=\"green\"><B>acc</B></FONT>".to_string(),
                    Action::Error => String::new(),
                };
            }
        }
    }
    String::new()
}

/// Format a goto table cell
fn format_goto_cell(parse_table: &ParseTable, state: usize, sym_id: u32) -> String {
    for ((s, symbol), &next_state) in &parse_table.goto_table {
        if *s == state && symbol_matches_id(symbol, sym_id) {
            return format!("{}", next_state);
        }
    }
    String::new()
}

/// Check if a symbol matches a given ID (simplified - uses name matching)
fn symbol_matches_id(symbol: &crate::grammar::Symbol, _sym_id: u32) -> bool {
    // This is a placeholder - in production, we'd need proper symbol->ID mapping
    // For now, we always return false and rely on direct iteration
    let _ = symbol;
    false
}

/// Escape special characters for HTML labels
fn escape_html(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}
