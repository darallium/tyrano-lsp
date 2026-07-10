//! Incremental reparsing with line-level green-subtree reuse.
//!
//! Contract: [`reparse`] is **always** structurally equivalent to a full
//! `parse_with_options(new_source, old.options())`, diagnostics included —
//! edits are only a reuse hint, never a semantic input. Correctness rests
//! on three facts:
//!
//! 1. Every top-level child of `SCENARIO` starts at a physical line whose
//!    lexer mode is `Default` (block constructs contain their non-default
//!    interior), so the parse of such a child is a pure function of its
//!    own text.
//! 2. Green nodes are position-independent (lengths only), so an old
//!    subtree can be spliced at a shifted offset unchanged.
//! 3. Parser diagnostics are derived from the finished tree, and lexical
//!    diagnostics come from a full lex of the new text, so reuse cannot
//!    change the diagnostic set.
//!
//! Strategy: compute the damaged region as the common-prefix/common-suffix
//! diff of old vs. new text (the caller's `TextEdit`s are not trusted),
//! map each old top-level subtree that lies entirely outside the damage to
//! its expected new offset, and let the parser splice any candidate whose
//! bytes are identical at that offset. The parser consults the oracle only
//! at top-level positions, so a candidate that would now be swallowed by a
//! new enclosing block (e.g. the user typed `[iscript]` above it) is never
//! offered a chance to leak into the wrong context.

use std::collections::HashMap;

use crate::green::{GreenElement, GreenNode};
use crate::kind::SyntaxKind;
use crate::parser::{Parse, parse_lexed_with_reuse};
use crate::text::{SourceText, TextEdit};

/// Old top-level subtrees keyed by the offset where they would start in
/// the new text.
pub(crate) struct ReuseMap {
    old_source: SourceText,
    /// new_start → (old_start, subtree)
    candidates: HashMap<usize, (usize, GreenNode)>,
}

impl ReuseMap {
    /// Returns a subtree that may be spliced at `new_start`, verifying
    /// that the new text is byte-identical to the old subtree's text.
    ///
    /// Only a subtree whose old parse was *self-terminated* may be reused
    /// away from EOF. A subtree that ended exactly at the old EOF may have
    /// ended only *because* of EOF — a final line without `\n` would now
    /// continue with appended text, and an unterminated `[iscript]` /
    /// `/*` block would have swallowed any following lines. Such
    /// candidates are reusable only when they end exactly at the new EOF
    /// too. (Mid-file top-level subtrees always own a terminating newline
    /// and never depend on what follows.)
    pub(crate) fn reusable_at(&self, new_start: usize, new_source: &str) -> Option<GreenNode> {
        let (old_start, node) = self.candidates.get(&new_start)?;
        let len = node.full_len().to_usize();
        let old = self.old_source.as_str();
        let old_text = old.get(*old_start..old_start + len)?;
        let new_text = new_source.get(new_start..new_start + len)?;
        if old_text != new_text {
            return None;
        }
        let eof_terminated = old_start + len == old.len();
        if eof_terminated && new_start + len != new_source.len() {
            return None;
        }
        debug_assert!(
            eof_terminated || old_text.ends_with('\n'),
            "mid-file top-level subtrees must end with a newline"
        );
        Some(node.clone())
    }
}

/// Reparses `new_source`, reusing unchanged top-level subtrees from `old`.
pub(crate) fn reparse(old: &Parse, new_source: &str, _edits: &[TextEdit]) -> Parse {
    let old_source = old.source().as_str();
    let (damage_start, damage_end_old) = diff_bounds(old_source, new_source);
    let delta = new_source.len() as isize - old_source.len() as isize;

    let mut candidates = HashMap::new();
    let mut offset = 0usize;
    for child in old.green().children() {
        let len = child.full_len().to_usize();
        let (start, end) = (offset, offset + len);
        offset = end;
        let GreenElement::Node(node) = child else { continue };
        if !self_terminated(node) {
            continue;
        }
        if end <= damage_start {
            candidates.insert(start, (start, node.clone()));
        } else if start >= damage_end_old {
            let new_start = start as isize + delta;
            debug_assert!(new_start >= 0, "suffix children lie inside the new text");
            candidates.insert(new_start as usize, (start, node.clone()));
        }
    }

    let options = old.options().clone();
    let lexed = crate::lexer::lex(new_source, &crate::lexer::LexOptions {
        loose_endscript_termination: options.loose_endscript_termination,
    });
    let reuse =
        ReuseMap { old_source: old.source().clone(), candidates };
    parse_lexed_with_reuse(new_source, lexed, &options, Some(reuse))
}

/// Whether a subtree's extent is a pure function of its own text.
///
/// An `[iscript]`/`[html]` block that was closed by a *real* end-tag line
/// carries that closer as its last child node. A block whose last child is
/// a token was terminated from the outside — by the loose-endscript quirk
/// (the terminating line is a following sibling) or by EOF — so its extent
/// depends on what comes after it and it must never be reused mid-file.
/// Every other line construct owns its terminator.
fn self_terminated(node: &GreenNode) -> bool {
    match node.kind() {
        SyntaxKind::ISCRIPT_BLOCK | SyntaxKind::HTML_BLOCK => {
            matches!(node.children().last(), Some(GreenElement::Node(_)))
        }
        _ => true,
    }
}

/// `(damage_start, damage_end_old)`: the smallest old-text range outside
/// of which old and new text agree (common prefix / common suffix, with
/// the suffix clamped so the two regions never overlap).
fn diff_bounds(old: &str, new: &str) -> (usize, usize) {
    let prefix =
        old.bytes().zip(new.bytes()).take_while(|(a, b)| a == b).count();
    let max_suffix = old.len().min(new.len()) - prefix;
    let suffix = old
        .bytes()
        .rev()
        .zip(new.bytes().rev())
        .take_while(|(a, b)| a == b)
        .count()
        .min(max_suffix);
    (prefix, old.len() - suffix)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parser::{ParseOptions, parse, parse_with_options};
    use crate::red::SyntaxNode;
    use crate::text::{TextRange, TextSize};

    const CORPUS: &str = "\u{feff};オープニングシナリオ\n\
*start|セーブ1\n\
#akane:happy\n\
  こんにちは[l]世界[r]\n\
@bg storage=room.jpg time=1000\n\
/*\n\
  メモ: ここは隠す\n\
*/\n\
[iscript]\n\
var a = 1;\n\
f.name = \"あかね\";\n\
[endscript]\n\
_  インデント保持テキスト\n\
[macro_use * flag2]\n\
\n\
*end\n";

    fn edit(src: &str, range: std::ops::Range<usize>, replacement: &str) -> (String, TextEdit) {
        let mut new = String::new();
        new.push_str(&src[..range.start]);
        new.push_str(replacement);
        new.push_str(&src[range.end..]);
        let te = TextEdit::replace(
            TextRange::new(TextSize::new(range.start as u32), TextSize::new(range.end as u32)),
            replacement.to_string(),
        );
        (new, te)
    }

    /// The incremental contract: tree AND diagnostics identical to a full
    /// parse.
    fn check_equivalent(old: &Parse, new_source: &str, edits: &[TextEdit]) -> Parse {
        let inc = old.reparse(new_source, edits);
        let full = parse_with_options(new_source, old.options());
        assert_eq!(inc.green(), full.green(), "tree must equal full reparse");
        assert_eq!(inc.diagnostics(), full.diagnostics(), "diagnostics must match");
        assert_eq!(inc.to_source(), new_source, "round-trip must hold");
        inc
    }

    /// Count top-level children of `new` that are pointer-shared with `old`
    /// (i.e. actually reused, not rebuilt).
    fn shared_top_level(old: &Parse, new: &Parse) -> usize {
        let olds: Vec<GreenNode> = old
            .green()
            .children()
            .iter()
            .filter_map(|c| match c {
                GreenElement::Node(n) => Some(n.clone()),
                GreenElement::Token(_) => None,
            })
            .collect();
        new.green()
            .children()
            .iter()
            .filter_map(|c| match c {
                GreenElement::Node(n) => Some(n),
                GreenElement::Token(_) => None,
            })
            .filter(|n| olds.iter().any(|o| GreenNode::ptr_eq(o, n)))
            .count()
    }

    #[test]
    fn edit_at_end_reuses_prefix() {
        let old = parse(CORPUS);
        let pos = CORPUS.rfind("*end").unwrap();
        let (new, te) = edit(CORPUS, pos..pos + 4, "*fin");
        let inc = check_equivalent(&old, &new, &[te]);
        assert!(shared_top_level(&old, &inc) >= 7, "prefix lines must be reused");
    }

    #[test]
    fn edit_at_start_reuses_suffix() {
        let old = parse(CORPUS);
        let pos = CORPUS.find("オープニング").unwrap();
        let (new, te) = edit(CORPUS, pos..pos, "改:");
        let inc = check_equivalent(&old, &new, &[te]);
        assert!(shared_top_level(&old, &inc) >= 7, "suffix lines must be reused");
    }

    #[test]
    fn edit_in_middle_reuses_both_sides() {
        let old = parse(CORPUS);
        let pos = CORPUS.find("こんにちは").unwrap();
        let (new, te) = edit(CORPUS, pos..pos + "こんにちは".len(), "さようなら");
        let inc = check_equivalent(&old, &new, &[te]);
        assert!(shared_top_level(&old, &inc) >= 6);
    }

    #[test]
    fn edit_inside_iscript_rebuilds_only_the_block() {
        let old = parse(CORPUS);
        let pos = CORPUS.find("var a = 1;").unwrap();
        let (new, te) = edit(CORPUS, pos..pos + 10, "var a = 2;");
        let inc = check_equivalent(&old, &new, &[te]);
        // The block itself must be rebuilt, everything else reusable.
        assert!(shared_top_level(&old, &inc) >= 6);
    }

    #[test]
    fn opening_a_block_above_suffix_swallows_it_correctly() {
        // Typing `[iscript]` must prevent stale reuse of the lines below
        // (they are script text now).
        let old = parse("aaa\nbbb\nccc\n");
        let (new, te) = edit("aaa\nbbb\nccc\n", 4..4, "[iscript]\n");
        let inc = check_equivalent(&old, &new, &[te]);
        let root = SyntaxNode::new_root(inc.green().clone());
        let kinds: Vec<_> = root.children().map(|c| c.kind()).collect();
        assert!(kinds.contains(&crate::SyntaxKind::ISCRIPT_BLOCK));
    }

    #[test]
    fn closing_quote_ripples_forward() {
        // Removing the closing bracket of a tag changes the whole line's
        // parse but not its neighbours.
        let old = parse("[a]\n[b]\n[c]\n");
        let (new, te) = edit("[a]\n[b]\n[c]\n", 5..6, "");
        check_equivalent(&old, &new, &[te]);
    }

    #[test]
    fn identical_reparse_reuses_everything() {
        let old = parse(CORPUS);
        let inc = check_equivalent(&old, CORPUS, &[]);
        let total = old
            .green()
            .children()
            .iter()
            .filter(|c| matches!(c, GreenElement::Node(_)))
            .count();
        assert_eq!(shared_top_level(&old, &inc), total);
    }

    #[test]
    fn options_are_inherited() {
        let strict = ParseOptions { loose_endscript_termination: false };
        let src = "[iscript]\nvar s = \"endscript\";\n";
        let old = parse_with_options(src, &strict);
        let (new, te) = edit(src, 0..0, ";c\n");
        let inc = check_equivalent(&old, &new, &[te]);
        assert_eq!(inc.options(), &strict);
    }

    #[test]
    fn fuzz_lite_random_edits_stay_consistent() {
        // Deterministic LCG-driven edit storm over the corpus.
        let mut state: u64 = 0x243F_6A88_85A3_08D3;
        let mut rnd = move |m: usize| {
            state = state.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
            ((state >> 33) as usize) % m.max(1)
        };
        let snippets =
            ["[l]", "\n", "*x|y", "@bg t=1", "[iscript]", "endscript", "\"", "]", "あ", ";c", "/*"];
        let mut src = CORPUS.to_string();
        let mut parsed = parse(&src);
        for _ in 0..60 {
            // Pick a char-boundary-safe random range.
            let mut a = rnd(src.len() + 1);
            while !src.is_char_boundary(a) {
                a -= 1;
            }
            let mut b = (a + rnd(12)).min(src.len());
            while !src.is_char_boundary(b) {
                b -= 1;
            }
            let (a, b) = (a.min(b), a.max(b));
            let (new, te) = edit(&src, a..b, snippets[rnd(snippets.len())]);
            parsed = check_equivalent(&parsed, &new, &[te]);
            src = new;
        }
    }
}
