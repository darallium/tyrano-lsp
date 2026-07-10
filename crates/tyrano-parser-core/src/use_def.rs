//! References and their resolutions (use → def and def → uses).

use tyrano_syntax::text::TextRange;

use crate::ast_id::ErasedAstId;
use crate::symbol::DefinitionId;

/// Index of a [`Reference`] in a [`UseDefMap`], in document order.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct UseId(pub(crate) u32);

impl UseId {
    /// The arena index.
    pub fn index(self) -> usize {
        self.0 as usize
    }
}

/// What kind of reference a use site is.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum RefKind {
    /// `target=*label` on `jump`/`call`/`link`/`button`.
    JumpTarget,
    /// A tag whose name matches a file-local `[macro name=…]` definition.
    MacroCall,
    /// `name=` on `chara_*` tags matching a local character definition.
    CharacterRef,
}

/// Where a reference points.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Resolution {
    /// Resolved to a file-local definition.
    Def(DefinitionId),
    /// Explicitly out of file-local scope: a jump with a non-empty
    /// `storage=`. Recorded for the future multi-file layer; not an error.
    External {
        storage: String,
        target: Option<String>,
    },
    /// Looks like a local reference but nothing matches. Produces a
    /// semantic error for [`RefKind::JumpTarget`]; silently recorded for
    /// character refs (characters are routinely defined in other files).
    Unknown,
}

/// One reference site.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Reference {
    pub kind: RefKind,
    /// Host item (the tag node).
    pub node: ErasedAstId,
    /// Range of the referencing text (param value / tag name).
    pub range: TextRange,
    /// Referenced name as written (label name without `*`, macro name,
    /// chara name). Empty for external refs without a `*label` target.
    pub name: String,
    pub resolution: Resolution,
}

/// Use → def and def → uses, both directions.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct UseDefMap {
    uses: Vec<Reference>,
    /// Indexed by `DefinitionId`; lists resolved uses in document order.
    def_to_uses: Vec<Vec<UseId>>,
}

impl UseDefMap {
    /// All references in document order (`UseId` = index).
    pub fn uses(&self) -> &[Reference] {
        &self.uses
    }

    pub fn use_(&self, u: UseId) -> &Reference {
        &self.uses[u.index()]
    }

    pub fn resolution(&self, u: UseId) -> &Resolution {
        &self.uses[u.index()].resolution
    }

    /// Uses that resolved to `def`, in document order.
    pub fn uses_of(&self, def: DefinitionId) -> &[UseId] {
        self.def_to_uses.get(def.index()).map(Vec::as_slice).unwrap_or(&[])
    }

    /// Appends a reference, maintaining the back-map. `def_count` is the
    /// current number of definitions in the index (sizes the back-map).
    pub(crate) fn push(&mut self, reference: Reference, def_count: usize) -> UseId {
        let id = UseId(self.uses.len() as u32);
        if let Resolution::Def(def) = &reference.resolution {
            if self.def_to_uses.len() < def_count {
                self.def_to_uses.resize(def_count, Vec::new());
            }
            self.def_to_uses[def.index()].push(id);
        }
        self.uses.push(reference);
        id
    }
}
