//! LR Automaton visualization using Graphviz DOT format

use crate::grammar::GrammarParser;
use crate::state::LRState;

/// Generate a Graphviz DOT representation of the LR automaton
pub fn generate_automaton_dot(states: &[LRState], grammar: &GrammarParser) -> String {
    let mut dot = String::new();

    dot.push_str("digraph LR_Automaton {\n");
    dot.push_str("    rankdir=LR;\n");
    dot.push_str("    node [shape=box, fontname=\"Courier New\", fontsize=10];\n");
    dot.push_str("    edge [fontname=\"Helvetica\", fontsize=9];\n");
    dot.push_str("\n");

    // Mark initial state
    dot.push_str("    start [shape=point, width=0.1];\n");
    dot.push_str("    start -> 0;\n\n");

    // Generate nodes for each state
    for state in states {
        let label = format_state_label(state, grammar);
        dot.push_str(&format!("    {} [label=<{}>];\n", state.id, label));
    }

    dot.push_str("\n");

    // Generate edges for transitions
    for state in states {
        for (symbol, &target_state) in &state.transitions {
            let symbol_name = escape_dot_label(symbol.name());
            let edge_color = if symbol.is_terminal() {
                "blue"
            } else {
                "darkgreen"
            };
            dot.push_str(&format!(
                "    {} -> {} [label=\"{}\", color={}];\n",
                state.id, target_state, symbol_name, edge_color
            ));
        }
    }

    dot.push_str("}\n");
    dot
}

/// Format LR items for a state as HTML-like label
fn format_state_label(state: &LRState, _grammar: &GrammarParser) -> String {
    let mut label = String::new();

    label.push_str("<TABLE BORDER=\"0\" CELLBORDER=\"1\" CELLSPACING=\"0\">");
    label.push_str(&format!(
        "<TR><TD BGCOLOR=\"lightgray\"><B>State {}</B></TD></TR>",
        state.id
    ));

    // Show only kernel items (items with dot not at position 0, or start symbol)
    let kernel_items: Vec<_> = state
        .items
        .iter()
        .filter(|item| item.dot_position > 0 || item.production.lhs.name().starts_with("Start"))
        .take(10) // Limit to prevent huge labels
        .collect();

    let total_items = state.items.len();
    let shown_items = kernel_items.len();

    for item in kernel_items {
        let item_str = format_lr_item(item);
        label.push_str(&format!("<TR><TD ALIGN=\"LEFT\">{}</TD></TR>", item_str));
    }

    if shown_items < total_items {
        label.push_str(&format!(
            "<TR><TD ALIGN=\"LEFT\"><I>... and {} more items</I></TD></TR>",
            total_items - shown_items
        ));
    }

    label.push_str("</TABLE>");
    label
}

/// Format a single LR item for display
fn format_lr_item(item: &crate::state::LRItem) -> String {
    let mut result = String::new();

    result.push_str(&escape_html(item.production.lhs.name()));
    result.push_str(" → ");

    if item.production.is_epsilon_production() {
        result.push_str("• ε");
    } else {
        for (i, symbol) in item.production.rhs.iter().enumerate() {
            if i == item.dot_position {
                result.push_str("• ");
            }
            result.push_str(&escape_html(symbol.name()));
            result.push(' ');
        }
        if item.dot_position >= item.production.rhs.len() {
            result.push_str("•");
        }
    }

    result.push_str(&format!(", {}", escape_html(item.lookahead.name())));
    result
}

/// Escape special characters for DOT labels
fn escape_dot_label(s: &str) -> String {
    s.replace('\\', "\\\\")
        .replace('"', "\\\"")
        .replace('<', "\\<")
        .replace('>', "\\>")
        .replace('|', "\\|")
        .replace('{', "\\{")
        .replace('}', "\\}")
}

/// Escape special characters for HTML labels
fn escape_html(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}
