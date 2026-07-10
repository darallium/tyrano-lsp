//! Salsa-compatible parse result.
//!
//! [`tyrano_syntax::Parse`] itself cannot be a salsa value: it carries a
//! `SourceText`, which is not `Eq`. [`ParsedModule`] keeps only the
//! position-independent, structurally comparable pieces — the green tree,
//! the out-of-tree diagnostics, and the options the tree was parsed with —
//! and rebuilds ephemeral red cursors on demand.

use std::sync::Arc;

use tyrano_syntax::ParseOptions;
use tyrano_syntax::ast::{AstNode as _, Scenario};
use tyrano_syntax::diagnostics::Diagnostic;
use tyrano_syntax::green::GreenNode;
use tyrano_syntax::red::SyntaxNode;

/// The memoizable result of parsing one scenario file.
///
/// Cloning is cheap (two `Arc`s plus a flag). Equality is structural:
/// [`GreenNode`] fast-rejects on its content hash, so salsa backdating
/// stops invalidation when an edit reparses to an identical tree.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParsedModule {
    green: GreenNode,
    diagnostics: Arc<[Diagnostic]>,
    options: ParseOptions,
}

impl ParsedModule {
    /// Extracts the durable pieces of a [`tyrano_syntax::Parse`].
    pub fn from_parse(parse: &tyrano_syntax::Parse) -> ParsedModule {
        ParsedModule {
            green: parse.green().clone(),
            diagnostics: parse.diagnostics().into(),
            options: parse.options().clone(),
        }
    }

    /// The durable green tree.
    pub fn green(&self) -> &GreenNode {
        &self.green
    }

    /// A fresh red root cursor. Red nodes are ephemeral and position-aware;
    /// never store them in salsa-visible state.
    pub fn syntax(&self) -> SyntaxNode {
        SyntaxNode::new_root(self.green.clone())
    }

    /// The typed AST root.
    pub fn scenario(&self) -> Scenario {
        Scenario::cast(self.syntax()).expect("parse root is always SCENARIO")
    }

    /// Lex/parse diagnostics, sorted by primary span. Semantic diagnostics
    /// live downstream in `tyrano-parser-core`.
    pub fn diagnostics(&self) -> &[Diagnostic] {
        &self.diagnostics
    }

    /// The options the file was parsed with.
    pub fn options(&self) -> &ParseOptions {
        &self.options
    }
}

// `GreenNode`/`Diagnostic` are foreign types without `salsa::Update`
// impls, so the derive is unavailable; for an `Eq` value type the
// equality-based impl is exactly what backdating needs.
unsafe impl salsa::Update for ParsedModule {
    unsafe fn maybe_update(old: *mut Self, new: Self) -> bool {
        let old = unsafe { &mut *old };
        if *old == new {
            false
        } else {
            *old = new;
            true
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tyrano_syntax::parse;

    const SRC: &str = "*start\nこんにちは[l]世界\n[jump target=*start]\n";

    #[test]
    fn roundtrips_green_tree() {
        let module = ParsedModule::from_parse(&parse(SRC));
        assert_eq!(module.green().to_source(), SRC);
        assert_eq!(module.syntax().text_range().len(), tyrano_syntax::text::TextSize::of(SRC));
    }

    #[test]
    fn eq_is_structural() {
        let a = ParsedModule::from_parse(&parse(SRC));
        let b = ParsedModule::from_parse(&parse(SRC));
        assert!(!GreenNode::ptr_eq(a.green(), b.green()), "independent parses");
        assert_eq!(a, b);
        let c = ParsedModule::from_parse(&parse("*other\n"));
        assert_ne!(a, c);
    }

    #[test]
    fn exposes_scenario_and_diags() {
        let parse = parse(SRC);
        let module = ParsedModule::from_parse(&parse);
        assert_eq!(module.scenario().lines().count(), 3);
        assert_eq!(module.diagnostics(), parse.diagnostics());
    }

    #[test]
    fn broken_input_still_yields_module_with_diagnostics() {
        let module = ParsedModule::from_parse(&parse("[unclosed\n"));
        assert!(!module.diagnostics().is_empty());
        assert_eq!(module.green().to_source(), "[unclosed\n");
    }
}
