use std::env;
use std::fs;
use std::path::Path;

fn main() {
    println!(
        "FROM_BUILDRS_STDOUT: {:?}",
        option_env!("FROM_BUILDRS_STDOUT")
    );
    println!(
        "FROM_BUILDRS_STDEER: {:?}",
        option_env!("FROM_BUILDRS_STDEER")
    );

    // 文法ファイルを読み込んでRustコードとして生成
    let out_dir = env::var("OUT_DIR").unwrap();
    println!("out_dir: {out_dir}");
    let grammar_dir = Path::new("../../grammar/");
    let grammar_path = grammar_dir.join(Path::new("tyranoscript.grammar"));

    println!("cargo:rerun-if-changed=../../grammar/tyranoscript.grammar");

    let grammar_content = fs::read_to_string(grammar_path).expect("Failed to read grammar file");

    // Generate debug info
    generate_debug_info(&grammar_content, &out_dir);

    let generated = generate_grammar_code(&grammar_content);

    let dest_path = Path::new(&out_dir).join("grammar_generated.rs");
    fs::write(&dest_path, generated).expect("Failed to write generated grammar");
}

fn generate_debug_info(grammar: &str, out_dir: &str) {
    let mut debug_output = String::new();

    debug_output.push_str("=== Grammar Debug Info ===\n\n");
    debug_output.push_str(&format!("Total lines: {}\n\n", grammar.lines().count()));

    debug_output.push_str("=== Token Declarations ===\n");
    let mut in_rules = false;
    for (i, line) in grammar.lines().enumerate() {
        let trimmed = line.trim();
        if trimmed == "%%" {
            in_rules = !in_rules;
            debug_output.push_str(&format!("Line {}: SEPARATOR (%%)\n", i + 1));
            continue;
        }

        if !in_rules && (trimmed.starts_with("%token") || trimmed.starts_with("%start")) {
            debug_output.push_str(&format!("Line {}: {}\n", i + 1, trimmed));
        }
    }

    debug_output.push_str("\n=== Production Rules ===\n");
    in_rules = false;
    let mut current_lhs = String::new();

    for (i, line) in grammar.lines().enumerate() {
        let trimmed = line.trim();

        if trimmed == "%%" {
            in_rules = !in_rules;
            continue;
        }

        if in_rules && !trimmed.is_empty() && !trimmed.starts_with("//") {
            if trimmed.ends_with(':') {
                current_lhs = trimmed[..trimmed.len() - 1].to_string();
                debug_output.push_str(&format!(
                    "\nLine {}: LHS (ends with colon): '{}'\n",
                    i + 1,
                    current_lhs
                ));
            } else if trimmed.starts_with(':') {
                debug_output.push_str(&format!(
                    "Line {}: RHS (starts with colon): '{}'\n",
                    i + 1,
                    trimmed
                ));
            } else if trimmed.starts_with('|') {
                debug_output.push_str(&format!(
                    "Line {}: RHS (alternative): '{}'\n",
                    i + 1,
                    trimmed
                ));
            } else if trimmed == ";" {
                debug_output.push_str(&format!("Line {}: END OF RULE\n", i + 1));
                current_lhs.clear();
            } else {
                // Bare non-terminal name
                debug_output.push_str(&format!("Line {}: BARE LHS: '{}'\n", i + 1, trimmed));
                current_lhs = trimmed.to_string();
            }
        }
    }

    let debug_path = Path::new(out_dir).join("grammar_debug.txt");
    fs::write(&debug_path, debug_output).expect("Failed to write debug info");

    println!(
        "cargo:warning=Debug info written to: {}",
        debug_path.display()
    );
}

fn generate_grammar_code(grammar: &str) -> String {
    format!(
        r####"
        pub const GRAMMAR_DEFINITION: &str = r###"{grammar}"###;
        "####
    )
}
