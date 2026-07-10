//! Integration tests over the real scenario corpus: the lossless
//! round-trip invariant, error tolerance, and incremental consistency.

use std::path::PathBuf;

use tyrano_syntax::text::{TextEdit, TextRange, TextSize};
use tyrano_syntax::{ParseOptions, parse, parse_with_options};

fn corpus() -> Vec<(String, String)> {
    let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../debug_artifacts");
    let mut files: Vec<_> = std::fs::read_dir(&dir)
        .expect("debug_artifacts directory")
        .filter_map(|e| {
            let path = e.ok()?.path();
            (path.extension()? == "ks").then(|| {
                let text = std::fs::read_to_string(&path).expect("readable corpus file");
                (path.file_name().unwrap().to_string_lossy().into_owned(), text)
            })
        })
        .collect();
    files.sort();
    assert!(!files.is_empty(), "corpus must not be empty");
    files
}

#[test]
fn corpus_roundtrips_byte_exact() {
    for (name, text) in corpus() {
        let parsed = parse(&text);
        assert_eq!(parsed.to_source(), text, "{name}: round-trip failed");
        for opts in [
            ParseOptions { loose_endscript_termination: true },
            ParseOptions { loose_endscript_termination: false },
        ] {
            let parsed = parse_with_options(&text, &opts);
            assert_eq!(parsed.to_source(), text, "{name}: round-trip failed ({opts:?})");
        }
    }
}

#[test]
fn corpus_truncations_never_panic_and_roundtrip() {
    // Truncating at every char boundary simulates half-typed files.
    for (name, text) in corpus() {
        for (i, _) in text.char_indices().step_by(7) {
            let cut = &text[..i];
            let parsed = parse(cut);
            assert_eq!(parsed.to_source(), cut, "{name}[..{i}]: round-trip failed");
        }
    }
}

#[test]
fn corpus_incremental_edits_match_full_parse() {
    for (name, text) in corpus() {
        let old = parse(&text);
        // A few representative edits: prepend, append, replace the middle.
        let mid = {
            let mut m = text.len() / 2;
            while !text.is_char_boundary(m) {
                m -= 1;
            }
            m
        };
        let cases = [
            (0usize, 0usize, ";inserted comment\n"),
            (text.len(), text.len(), "\n*added\n"),
            (mid, mid, "[iscript]\nvar x=1;\n[endscript]\n"),
        ];
        for (a, b, ins) in cases {
            let mut new = String::new();
            new.push_str(&text[..a]);
            new.push_str(ins);
            new.push_str(&text[b..]);
            let edit = TextEdit::replace(
                TextRange::new(TextSize::new(a as u32), TextSize::new(b as u32)),
                ins.to_string(),
            );
            let inc = old.reparse(&new, &[edit]);
            let full = parse(&new);
            assert_eq!(inc.green(), full.green(), "{name}: incremental tree diverged");
            assert_eq!(
                inc.diagnostics(),
                full.diagnostics(),
                "{name}: incremental diagnostics diverged"
            );
        }
    }
}
