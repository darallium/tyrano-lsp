//! Deterministic robustness storm (no external fuzzer).
//!
//! Every generated input — structured fragment soup and random mutations
//! of the real corpus — must uphold the crate's core invariants:
//! no panic, byte-exact round-trip, full byte coverage, and incremental
//! reparse ≡ full parse (tree and diagnostics).

use tyrano_syntax::text::{TextEdit, TextRange, TextSize};
use tyrano_syntax::{Parse, ParseOptions, parse, parse_with_options};

/// Deterministic LCG (same recipe as the incremental unit tests).
struct Rng(u64);

impl Rng {
    fn next(&mut self, m: usize) -> usize {
        self.0 = self.0.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
        ((self.0 >> 33) as usize) % m.max(1)
    }
}

const FRAGMENTS: &[&str] = &[
    "[", "]", "[l]", "@bg t=", "\"", "'", "`", "\\", "*a|b", "#n:f", ";c", "/*", "*/",
    "[iscript]", "endscript", "[endscript]", "[html]", "[endhtml]", "&f.x+1", "%p", "\n",
    "\r\n", " ", "  ", "_", "あ", "漢字テキスト", "\u{feff}", "=", "|", ":", "*", "@", "#",
    "[eval exp=f.a[0]]", "t=undefined", "[macro_use * flag2]",
];

fn assert_invariants(src: &str) -> Parse {
    let parsed = parse(src);
    assert_eq!(parsed.to_source(), src, "round-trip failed for {src:?}");
    assert_eq!(parsed.green().full_len().to_usize(), src.len(), "coverage failed for {src:?}");
    parsed
}

#[test]
fn structured_soup() {
    let mut rng = Rng(0x9E37_79B9_7F4A_7C15);
    let strict = ParseOptions { loose_endscript_termination: false };
    for i in 0..300 {
        let mut src = String::new();
        for _ in 0..rng.next(60) {
            src.push_str(FRAGMENTS[rng.next(FRAGMENTS.len())]);
        }
        assert_invariants(&src);
        if i % 10 == 0 {
            let parsed = parse_with_options(&src, &strict);
            assert_eq!(parsed.to_source(), src, "strict round-trip failed for {src:?}");
        }
    }
}

#[test]
fn corpus_mutation_storm_with_incremental() {
    let dir = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../debug_artifacts");
    let mut rng = Rng(0x243F_6A88_85A3_08D3);
    for entry in std::fs::read_dir(dir).expect("corpus dir") {
        let path = entry.expect("entry").path();
        if path.extension().is_none_or(|e| e != "ks") {
            continue;
        }
        let mut src = std::fs::read_to_string(&path).expect("readable corpus file");
        let mut parsed = assert_invariants(&src);
        for _ in 0..200 {
            // Random char-boundary-safe replacement.
            let mut a = rng.next(src.len() + 1);
            while !src.is_char_boundary(a) {
                a -= 1;
            }
            let mut b = (a + rng.next(10)).min(src.len());
            while !src.is_char_boundary(b) {
                b -= 1;
            }
            let (a, b) = (a.min(b), a.max(b));
            let insert = FRAGMENTS[rng.next(FRAGMENTS.len())];

            let mut new = String::with_capacity(src.len() + insert.len());
            new.push_str(&src[..a]);
            new.push_str(insert);
            new.push_str(&src[b..]);

            let edit = TextEdit::replace(
                TextRange::new(TextSize::new(a as u32), TextSize::new(b as u32)),
                insert.to_string(),
            );
            let inc = parsed.reparse(&new, &[edit]);
            let full = assert_invariants(&new);
            if inc.green() != full.green() || inc.diagnostics() != full.diagnostics() {
                let dir = std::env::temp_dir();
                std::fs::write(dir.join("fuzz_old.ks"), &src).unwrap();
                std::fs::write(dir.join("fuzz_new.ks"), &new).unwrap();
                let dump = |p: &Parse| -> String {
                    let root = tyrano_syntax::red::SyntaxNode::new_root(p.green().clone());
                    root.children_with_tokens()
                        .map(|e| format!("{:?}@{:?}\n", e.kind(), e.text_range()))
                        .collect()
                };
                std::fs::write(dir.join("fuzz_inc.tree"), dump(&inc)).unwrap();
                std::fs::write(dir.join("fuzz_full.tree"), dump(&full)).unwrap();
                panic!(
                    "incremental divergence; dumps in {}/fuzz_{{old,new}}.ks + .tree",
                    dir.display()
                );
            }

            src = new;
            parsed = inc;
        }
    }
}
