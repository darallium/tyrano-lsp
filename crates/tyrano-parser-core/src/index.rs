//! The [`SemanticIndex`]: all file-local semantic facts for one parse.

use std::collections::HashMap;
use std::fmt::Write as _;

use tyrano_syntax::text::TextRange;

use crate::ast_id::ErasedAstId;
use crate::errors::SemanticError;
use crate::place::{AccessKind, Place};
use crate::scope::{Scope, ScopeId, ScopeKind};
use crate::symbol::{Definition, DefinitionId, Symbol, SymbolId, SymbolKind};
use crate::use_def::{Reference, Resolution, UseDefMap};

/// Index of a [`Place`] in a [`SemanticIndex`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct PlaceId(pub(crate) u32);

impl PlaceId {
    /// The arena index.
    pub fn index(self) -> usize {
        self.0 as usize
    }
}

/// One occurrence of a [`Place`] in the file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlaceOccurrence {
    pub place: PlaceId,
    /// The tag/param host item.
    pub node: ErasedAstId,
    /// File-absolute range of the path expression.
    pub range: TextRange,
    pub access: AccessKind,
}

/// The language embedded in a block construct.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum EmbeddedLang {
    /// `[iscript] … [endscript]`: JavaScript.
    IScript,
    /// `[html] … [endhtml]`: HTML.
    Html,
}

/// One embedded foreign-language region.
///
/// The block interiors are a different language (JS/HTML) and are not
/// analyzed here; recording where they are is the file-local fact an IDE
/// layer needs for highlighting and for delegating to embedded tooling.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EmbeddedRegion {
    pub lang: EmbeddedLang,
    /// The `ISCRIPT_BLOCK` / `HTML_BLOCK` item.
    pub block: ErasedAstId,
    /// Range of the embedded code itself (excluding the delimiter tags);
    /// empty (anchored after the opening tag) when the block has no code.
    pub code_range: TextRange,
}

/// All file-local semantic facts for one parse revision.
///
/// Arenas are indexed by their id types; `*_by_name` lookups follow the
/// engine's first-definition-wins rule (they point at the symbol whose
/// `defs[0]` is the winning site).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SemanticIndex {
    pub(crate) symbols: Vec<Symbol>,
    pub(crate) definitions: Vec<Definition>,
    pub(crate) scopes: Vec<Scope>,
    pub(crate) places: Vec<Place>,
    pub(crate) place_occurrences: Vec<PlaceOccurrence>,
    pub(crate) embedded_regions: Vec<EmbeddedRegion>,
    pub(crate) use_def: UseDefMap,
    pub(crate) errors: Vec<SemanticError>,
    pub(crate) labels_by_name: HashMap<String, SymbolId>,
    pub(crate) macros_by_name: HashMap<String, SymbolId>,
    pub(crate) charas_by_name: HashMap<String, SymbolId>,
    pub(crate) place_lookup: HashMap<Place, PlaceId>,
    pub(crate) scope_by_item: HashMap<ErasedAstId, ScopeId>,
}

impl SemanticIndex {
    pub fn symbols(&self) -> &[Symbol] {
        &self.symbols
    }

    pub fn symbol(&self, id: SymbolId) -> &Symbol {
        &self.symbols[id.index()]
    }

    pub fn definitions(&self) -> &[Definition] {
        &self.definitions
    }

    pub fn definition(&self, id: DefinitionId) -> &Definition {
        &self.definitions[id.index()]
    }

    pub fn scopes(&self) -> &[Scope] {
        &self.scopes
    }

    pub fn scope(&self, id: ScopeId) -> &Scope {
        &self.scopes[id.index()]
    }

    /// The label symbol for `name` (first definition wins), if any.
    pub fn label(&self, name: &str) -> Option<SymbolId> {
        self.labels_by_name.get(name).copied()
    }

    /// The macro symbol for `name`, if any.
    pub fn macro_(&self, name: &str) -> Option<SymbolId> {
        self.macros_by_name.get(name).copied()
    }

    /// The character symbol for `name`, if any.
    pub fn character(&self, name: &str) -> Option<SymbolId> {
        self.charas_by_name.get(name).copied()
    }

    pub fn use_def(&self) -> &UseDefMap {
        &self.use_def
    }

    /// File-local semantic errors, sorted by `(start, end)`.
    pub fn errors(&self) -> &[SemanticError] {
        &self.errors
    }

    pub fn places(&self) -> &[Place] {
        &self.places
    }

    pub fn place(&self, id: PlaceId) -> &Place {
        &self.places[id.index()]
    }

    pub fn place_id(&self, place: &Place) -> Option<PlaceId> {
        self.place_lookup.get(place).copied()
    }

    /// All place occurrences in document order.
    pub fn place_occurrences(&self) -> &[PlaceOccurrence] {
        &self.place_occurrences
    }

    /// Occurrences of one place, in document order.
    pub fn occurrences_of(&self, id: PlaceId) -> impl Iterator<Item = &PlaceOccurrence> {
        self.place_occurrences.iter().filter(move |o| o.place == id)
    }

    /// Embedded foreign-language regions in document order.
    pub fn embedded_regions(&self) -> &[EmbeddedRegion] {
        &self.embedded_regions
    }

    /// The scope an item belongs to (defaults to the file root for items
    /// the builder did not attribute, e.g. ids from nested block tags).
    pub fn scope_of(&self, item: ErasedAstId) -> ScopeId {
        self.scope_by_item.get(&item).copied().unwrap_or(ScopeId::ROOT)
    }

    /// Deterministic, line-oriented dump for golden tests.
    pub fn debug_dump(&self) -> String {
        fn range(r: TextRange) -> String {
            format!("{}..{}", u32::from(r.start()), u32::from(r.end()))
        }

        let mut out = String::new();

        out.push_str("== scopes ==\n");
        for (i, scope) in self.scopes.iter().enumerate() {
            match &scope.kind {
                ScopeKind::Root => {
                    let _ = writeln!(out, "scope{i} root {}", range(scope.range));
                }
                ScopeKind::MacroBody { def } => {
                    let name = &self.symbol(self.definition(*def).symbol).name;
                    let parent = scope.parent.map_or("-".to_string(), |p| format!("scope{}", p.index()));
                    let _ = writeln!(
                        out,
                        "scope{i} macro-body(def{} \"{name}\") {} parent={parent}",
                        def.index(),
                        range(scope.range),
                    );
                }
            }
        }

        out.push_str("== symbols ==\n");
        for (i, sym) in self.symbols.iter().enumerate() {
            let kind = match sym.kind {
                SymbolKind::Label => "label",
                SymbolKind::Macro => "macro",
                SymbolKind::Character => "character",
            };
            let defs: Vec<String> = sym.defs.iter().map(|d| format!("def{}", d.index())).collect();
            let _ = writeln!(out, "sym{i} {kind} \"{}\" defs=[{}]", sym.name, defs.join(", "));
        }

        out.push_str("== definitions ==\n");
        for (i, def) in self.definitions.iter().enumerate() {
            let _ = writeln!(
                out,
                "def{i} sym{} item{} name@{} full@{} scope{}",
                def.symbol.index(),
                def.node.index(),
                range(def.name_range),
                range(def.full_range),
                def.scope.index(),
            );
        }

        out.push_str("== uses ==\n");
        for (i, use_) in self.use_def.uses().iter().enumerate() {
            let kind = match use_.kind {
                crate::use_def::RefKind::JumpTarget => "jump-target",
                crate::use_def::RefKind::MacroCall => "macro-call",
                crate::use_def::RefKind::CharacterRef => "character-ref",
            };
            let resolution = match &use_.resolution {
                Resolution::Def(d) => format!("def{}", d.index()),
                Resolution::External { storage, target } => match target {
                    Some(target) => format!("external(storage=\"{storage}\", target=\"{target}\")"),
                    None => format!("external(storage=\"{storage}\")"),
                },
                Resolution::Unknown => "unknown".to_string(),
            };
            let _ = writeln!(
                out,
                "use{i} {kind} \"{}\" @{} item{} -> {resolution}",
                use_.name,
                range(use_.range),
                use_.node.index(),
            );
        }

        out.push_str("== places ==\n");
        for (i, place) in self.places.iter().enumerate() {
            let _ = writeln!(out, "place{i} {place}");
        }
        for occ in &self.place_occurrences {
            let access = match occ.access {
                AccessKind::Read => "read",
                AccessKind::Write => "write",
            };
            let _ = writeln!(
                out,
                "occ place{} {access} @{} item{}",
                occ.place.index(),
                range(occ.range),
                occ.node.index(),
            );
        }

        out.push_str("== embedded ==\n");
        for region in &self.embedded_regions {
            let lang = match region.lang {
                EmbeddedLang::IScript => "iscript",
                EmbeddedLang::Html => "html",
            };
            let _ = writeln!(
                out,
                "embedded {lang} @{} item{}",
                range(region.code_range),
                region.block.index(),
            );
        }

        out.push_str("== errors ==\n");
        for err in &self.errors {
            let _ = writeln!(out, "{} @{} :: {}", err.code(), range(err.range), err.message());
        }

        out
    }

    /// A reference to `use_def::Reference` re-exported for convenience.
    pub fn uses(&self) -> &[Reference] {
        self.use_def.uses()
    }
}
