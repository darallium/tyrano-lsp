//! Document symbols: labels, macros, and characters of one file.

use tyrano_parser_core::{SymbolKind, semantic_index};
use tyrano_project::{File, ProjectDb};
use tyrano_syntax::text::TextRange;

/// One symbol of a document outline.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DocSymbol {
    pub name: String,
    pub kind: SymbolKind,
    /// The name range (selection range).
    pub range: TextRange,
    /// The full defining-node range.
    pub full_range: TextRange,
}

/// All definitions of `file` in document order.
pub fn document_symbols(db: &dyn ProjectDb, file: File) -> Vec<DocSymbol> {
    let index = semantic_index(db, file.source(db));
    let mut out: Vec<DocSymbol> = index
        .symbols()
        .iter()
        .flat_map(|symbol| {
            symbol.defs.iter().map(|&def| {
                let def = index.definition(def);
                DocSymbol {
                    name: symbol.name.clone(),
                    kind: symbol.kind,
                    range: def.name_range,
                    full_range: def.full_range,
                }
            })
        })
        .collect();
    out.sort_by_key(|s| (s.range.start(), s.range.end()));
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ide::testutil::{file, project};

    #[test]
    fn symbols_in_document_order() {
        let db = project(&[(
            "data/scenario/a.ks",
            "*start\n#akane\n[macro name=greet][endmacro]\n*end\n",
        )]);
        let f = file(&db, "data/scenario/a.ks");
        let symbols = document_symbols(&db, f);
        let names: Vec<(&str, SymbolKind)> =
            symbols.iter().map(|s| (s.name.as_str(), s.kind)).collect();
        assert_eq!(
            names,
            [
                ("start", SymbolKind::Label),
                ("akane", SymbolKind::Character),
                ("greet", SymbolKind::Macro),
                ("end", SymbolKind::Label),
            ]
        );
        assert!(symbols[0].full_range.contains_range(symbols[0].range));
    }
}
