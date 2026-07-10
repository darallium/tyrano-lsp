//! Named file-local symbols and their definition sites.
//!
//! Labels, macros, and characters get [`Symbol`] entries; variables do not
//! (they are dynamic, namespaced object paths with no single definition
//! site — see [`crate::place`]).

use tyrano_syntax::text::TextRange;

use crate::ast_id::ErasedAstId;
use crate::scope::ScopeId;

/// Index of a [`Symbol`] in [`crate::index::SemanticIndex`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct SymbolId(pub(crate) u32);

impl SymbolId {
    /// The arena index.
    pub fn index(self) -> usize {
        self.0 as usize
    }
}

/// Index of a [`Definition`] in [`crate::index::SemanticIndex`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct DefinitionId(pub(crate) u32);

impl DefinitionId {
    /// The arena index.
    pub fn index(self) -> usize {
        self.0 as usize
    }
}

/// Which namespace a symbol lives in.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum SymbolKind {
    /// `*name` jump target.
    Label,
    /// `[macro name=…]` definition.
    Macro,
    /// `#name` line or `[chara_new name=…]`.
    Character,
}

/// One named file-local symbol.
///
/// Duplicate definitions are legal input (the engine keeps running);
/// `defs[0]` is the winning definition (first-definition-wins).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Symbol {
    pub kind: SymbolKind,
    pub name: String,
    /// All definition sites in document order; never empty.
    pub defs: Vec<DefinitionId>,
}

/// One definition site.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Definition {
    pub symbol: SymbolId,
    /// The defining item node (`LABEL_LINE`, the `[macro]` tag,
    /// `CHARA_LINE` / `[chara_new]` tag).
    pub node: ErasedAstId,
    /// Range of the name itself (diagnostic/rename anchor): label name
    /// token, `name=` param value, chara name segment. Falls back to the
    /// full node range when the name token is missing.
    pub name_range: TextRange,
    /// Full range of the defining node.
    pub full_range: TextRange,
    pub scope: ScopeId,
}
