//! Lexical scopes.
//!
//! TyranoScript is almost flat: the only nesting construct is
//! `[macro] … [endmacro]`, which delimits a *sequence* of lines rather
//! than a subtree, and macros do not nest in the engine. Scopes are
//! therefore computed by a linear scan: the file root plus one
//! depth-1 scope per macro body.

use tyrano_syntax::text::TextRange;

use crate::symbol::DefinitionId;

/// Index of a [`Scope`] in [`crate::index::SemanticIndex`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct ScopeId(pub(crate) u32);

impl ScopeId {
    /// The file scope; always present at index 0.
    pub const ROOT: ScopeId = ScopeId(0);

    /// The arena index.
    pub fn index(self) -> usize {
        self.0 as usize
    }
}

/// What kind of region a scope covers.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ScopeKind {
    /// The file itself.
    Root,
    /// Between a `[macro]` tag and its `[endmacro]` (or EOF when
    /// unclosed). `mp.*` places and `%param` refs inside it belong here.
    MacroBody {
        /// The macro definition that opens this body.
        def: DefinitionId,
    },
}

/// One scope.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Scope {
    pub kind: ScopeKind,
    /// `MacroBody` → `Root`; `Root` has no parent.
    pub parent: Option<ScopeId>,
    /// Source range covered. For a macro body: from after the `[macro]`
    /// tag to the end of `[endmacro]` (or EOF).
    pub range: TextRange,
}
