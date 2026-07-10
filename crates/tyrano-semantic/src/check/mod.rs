//! Project-level checking: tag/parameter validation and cross-file
//! reference checks. Consumes the data-only registry in
//! `tyrano_project::registry`; produces `tyrano_db::FileDiagnostic`s with
//! the codes in [`crate::codes`].

pub mod kind;
pub(crate) mod tags;
pub(crate) mod xfile;

pub use kind::{KindMismatch, ValueClass, check_static, classify};

use tyrano_syntax::ast::{AnyTag, Line, Scenario, TextSegment};

/// Every checkable tag of a scenario in document order: `@tag` lines and
/// inline `[tag]`s. Mirrors the index builder's walk — `Line::Error`
/// skipped wholesale, block interiors (`[iscript]`/`[html]`) not
/// descended into.
pub(crate) fn all_tags(scenario: &Scenario) -> Vec<AnyTag> {
    let mut out = Vec::new();
    for line in scenario.lines() {
        match line {
            Line::AtTag(tag) => out.push(AnyTag::At(tag)),
            Line::Text(text) => out.extend(text.segments().into_iter().filter_map(|seg| {
                match seg {
                    TextSegment::Tag(tag) => Some(AnyTag::Inline(tag)),
                    TextSegment::Text(_) => None,
                }
            })),
            _ => {}
        }
    }
    out
}
