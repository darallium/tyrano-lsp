//! Position-independent, stable(-ish) node identity for one parsed file.
//!
//! A [`tyrano_syntax::red::SyntaxNodePtr`] is `(kind, range)` and therefore
//! *position-dependent*: any edit that shifts a node's byte offset changes its
//! pointer, even when the node's logical place in the file is unchanged.
//!
//! An [`AstId`] identifies a node by its DFS pre-order *ordinal* among
//! "item-like" nodes (see [`is_item_like`]) instead of its byte range. That
//! ordinal survives edits which only move offsets around: inserting blank lines
//! above an item changes every following item's range but not its ordinal, as
//! long as the *sequence* of items is unchanged.
//!
//! The mapping is captured for a single tree revision by [`AstIdMap`], which
//! stores one [`SyntaxNodePtr`] per item-like node, indexed by ordinal. Given a
//! fresh tree from a reparse, [`AstIdMap::resolve`] walks the new root to
//! recover "the same" node from a stored id.

use std::marker::PhantomData;

use tyrano_syntax::SyntaxKind;
use tyrano_syntax::ast::AstNode;
use tyrano_syntax::red::{SyntaxNode, SyntaxNodePtr};

/// Whether nodes of `kind` participate in the [`AstIdMap`] numbering.
///
/// These are the "item-like" nodes: the `SCENARIO` root plus the line- and
/// block-level constructs that a semantic layer wants stable handles to.
/// Structural/leaf nodes (`PARAM`, `TAG_NAME`, `LABEL_NAME`, ...) are excluded
/// so that ids stay attached to meaningful units.
pub(crate) fn is_item_like(kind: SyntaxKind) -> bool {
    matches!(
        kind,
        SyntaxKind::SCENARIO
            | SyntaxKind::TEXT_LINE
            | SyntaxKind::LABEL_LINE
            | SyntaxKind::CHARA_LINE
            | SyntaxKind::AT_TAG_LINE
            | SyntaxKind::INLINE_TAG
            | SyntaxKind::ISCRIPT_BLOCK
            | SyntaxKind::HTML_BLOCK
    )
}

/// A type-erased [`AstId`]: a DFS pre-order ordinal into an [`AstIdMap`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct ErasedAstId(u32);

impl ErasedAstId {
    /// Constructs an id from a raw ordinal. Crate-internal: ids are only
    /// meaningful relative to the [`AstIdMap`] that produced them.
    pub(crate) fn new(index: u32) -> ErasedAstId {
        ErasedAstId(index)
    }

    /// The raw ordinal, usable as a slice index.
    pub fn index(self) -> usize {
        self.0 as usize
    }
}

/// A typed [`ErasedAstId`], carrying the AST node type it points at.
///
/// The `PhantomData<fn() -> N>` tags the id with `N` without implying it owns
/// an `N`, and keeps `AstId<N>: Send + Sync + 'static` independent of `N`.
#[derive(Debug)]
pub struct AstId<N: AstNode> {
    raw: ErasedAstId,
    _ty: PhantomData<fn() -> N>,
}

// Manual impls: `#[derive]` would attach spurious `N: Clone`/`N: Eq`/... bounds,
// but an `AstId` is just a `u32` and is copyable/comparable for any `N`.
impl<N: AstNode> Clone for AstId<N> {
    fn clone(&self) -> Self {
        *self
    }
}

impl<N: AstNode> Copy for AstId<N> {}

impl<N: AstNode> PartialEq for AstId<N> {
    fn eq(&self, other: &Self) -> bool {
        self.raw == other.raw
    }
}

impl<N: AstNode> Eq for AstId<N> {}

impl<N: AstNode> std::hash::Hash for AstId<N> {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        self.raw.hash(state);
    }
}

impl<N: AstNode> AstId<N> {
    /// Drops the type tag, yielding the underlying [`ErasedAstId`].
    pub fn erased(self) -> ErasedAstId {
        self.raw
    }
}

/// Bidirectional map between [`ErasedAstId`]s and syntax nodes for **one** tree
/// revision.
///
/// The `ptrs` vector is indexed by ordinal (`ErasedAstId.0`) and lists every
/// item-like node in the tree in DFS pre-order. Because the tree is walked
/// pre-order and the root `SCENARIO` is itself item-like, the root is always
/// id `0`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AstIdMap {
    ptrs: Vec<SyntaxNodePtr>,
}

impl AstIdMap {
    /// Builds the map by collecting every item-like node reachable from `root`
    /// in DFS pre-order.
    ///
    /// `root` is expected to be a `SCENARIO` node; since `SCENARIO` is
    /// item-like and pre-order visits the root first, it receives id `0`.
    pub fn from_root(root: &SyntaxNode) -> AstIdMap {
        let ptrs = root
            .descendants()
            .filter(|node| is_item_like(node.kind()))
            .map(|node| SyntaxNodePtr::new(&node))
            .collect();
        AstIdMap { ptrs }
    }

    /// The id of `node`, matched by exact `(kind, range)`, or `None` if `node`
    /// is not an item-like node in this map's tree revision.
    pub fn erased_id(&self, node: &SyntaxNode) -> Option<ErasedAstId> {
        let ptr = SyntaxNodePtr::new(node);
        // Pre-order yields item-like nodes sorted by range start (ties: the
        // outer node first). A linear scan is simple and correct; the id count
        // per file is small.
        self.ptrs
            .iter()
            .position(|p| *p == ptr)
            .map(|i| ErasedAstId::new(i as u32))
    }

    /// The typed id of `node`.
    ///
    /// # Panics
    /// Panics if `node` is not present in this map (i.e. it is not an item-like
    /// node from this exact tree revision). Callers holding a node from the
    /// same tree used to build the map can rely on this succeeding.
    pub fn ast_id<N: AstNode>(&self, node: &N) -> AstId<N> {
        let raw = self
            .erased_id(node.syntax())
            .expect("node is not present in this AstIdMap");
        AstId {
            raw,
            _ty: PhantomData,
        }
    }

    /// The stored pointer for `id`.
    ///
    /// # Panics
    /// Panics if `id` is out of range for this map.
    pub fn ptr(&self, id: ErasedAstId) -> SyntaxNodePtr {
        self.ptrs[id.index()]
    }

    /// Resolves `id` against `root`, returning the live node if the stored
    /// pointer still matches something in `root`.
    pub fn resolve(&self, id: ErasedAstId, root: &SyntaxNode) -> Option<SyntaxNode> {
        self.ptr(id).resolve(root)
    }

    /// The number of item-like nodes (and hence ids) in this map.
    pub fn len(&self) -> usize {
        self.ptrs.len()
    }

    /// Whether the map holds no ids. Practically always `false`, since a
    /// `SCENARIO` root is itself item-like.
    pub fn is_empty(&self) -> bool {
        self.ptrs.is_empty()
    }

    /// Iterates `(id, ptr)` pairs in ascending id (DFS pre-order).
    pub fn iter(&self) -> impl Iterator<Item = (ErasedAstId, SyntaxNodePtr)> + '_ {
        self.ptrs
            .iter()
            .enumerate()
            .map(|(i, p)| (ErasedAstId::new(i as u32), *p))
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use super::*;
    use tyrano_syntax::ast::LabelLine;

    /// Fixture exercising each item-like construct: a label line, a text line
    /// with an inline `[l]` tag, an `@`-tag line, an `[iscript]` block, and an
    /// `[html]` block.
    const FIXTURE: &str = "*start\nこんにちは[l]世界\n@bg storage=room.jpg\n[iscript]\nvar a=1;\n[endscript]\n[html]\n<b>x</b>\n[endhtml]\n";

    /// The DFS pre-order kind sequence actually produced for [`FIXTURE`],
    /// verified by dumping the tree (see this module's history).
    ///
    /// Notable, non-obvious structure encoded here:
    /// - `AT_TAG_LINE` holds its `TAG_NAME`/`PARAM` directly; it does NOT wrap
    ///   an `INLINE_TAG`, so no id is emitted for the `@bg` tag itself.
    /// - `ISCRIPT_BLOCK`/`HTML_BLOCK` DO contain an `INLINE_TAG` child for their
    ///   *opening* tag (`[iscript]`, `[html]`).
    /// - The *closing* tag of each block (`[endscript]`, `[endhtml]`) is parsed
    ///   as a `TEXT_LINE` wrapping an `INLINE_TAG`, nested inside the block.
    fn expected_kinds() -> Vec<SyntaxKind> {
        use SyntaxKind::*;
        vec![
            SCENARIO,      // 0: root
            LABEL_LINE,    // 1: *start
            TEXT_LINE,     // 2: こんにちは[l]世界
            INLINE_TAG,    // 3:   [l] inside the text line
            AT_TAG_LINE,   // 4: @bg storage=room.jpg
            ISCRIPT_BLOCK, // 5: [iscript]...[endscript]
            INLINE_TAG,    // 6:   [iscript] opening tag
            TEXT_LINE,     // 7:   [endscript] closing tag (as a text line)
            INLINE_TAG,    // 8:     [endscript]
            HTML_BLOCK,    // 9: [html]...[endhtml]
            INLINE_TAG,    // 10:  [html] opening tag
            TEXT_LINE,     // 11:  [endhtml] closing tag (as a text line)
            INLINE_TAG,    // 12:    [endhtml]
        ]
    }

    #[test]
    fn scenario_root_is_id_zero() {
        let parse = tyrano_syntax::parse("*start\n");
        let map = AstIdMap::from_root(&parse.syntax());
        assert_eq!(map.ptr(ErasedAstId::new(0)).kind(), SyntaxKind::SCENARIO);
    }

    #[test]
    fn ids_follow_dfs_pre_order() {
        let parse = tyrano_syntax::parse(FIXTURE);
        let map = AstIdMap::from_root(&parse.syntax());

        let expected = expected_kinds();
        assert_eq!(map.len(), expected.len(), "unexpected item-like node count");

        let got: Vec<SyntaxKind> = (0..map.len())
            .map(|i| map.ptr(ErasedAstId::new(i as u32)).kind())
            .collect();
        assert_eq!(got, expected);
    }

    #[test]
    fn inline_tags_get_ids_params_do_not() {
        let parse = tyrano_syntax::parse("@bg storage=room.jpg name=v\nテキスト[l]\n");
        let root = parse.syntax();
        let map = AstIdMap::from_root(&root);

        for node in root.descendants() {
            match node.kind() {
                SyntaxKind::INLINE_TAG => {
                    assert!(
                        map.erased_id(&node).is_some(),
                        "every INLINE_TAG must have an id"
                    );
                }
                SyntaxKind::PARAM => {
                    assert!(
                        map.erased_id(&node).is_none(),
                        "PARAM nodes must not have an id"
                    );
                }
                _ => {}
            }
        }

        // Sanity: the fixture really does contain both kinds.
        assert!(root.descendants().any(|n| n.kind() == SyntaxKind::INLINE_TAG));
        assert!(root.descendants().any(|n| n.kind() == SyntaxKind::PARAM));
    }

    #[test]
    fn erased_id_roundtrips() {
        let parse = tyrano_syntax::parse(FIXTURE);
        let root = parse.syntax();
        let map = AstIdMap::from_root(&root);

        for node in root.descendants().filter(|n| is_item_like(n.kind())) {
            let id = map
                .erased_id(&node)
                .expect("item-like node must have an id");
            let resolved = map
                .resolve(id, &root)
                .expect("stored ptr must resolve in its own tree");
            assert_eq!(resolved.kind(), node.kind());
            assert_eq!(resolved.text_range(), node.text_range());
        }
    }

    #[test]
    fn ids_stable_under_leading_edit() {
        // Prepending blank lines shifts every range but must not change the
        // item sequence. (Verified separately: blank lines do NOT produce
        // TEXT_LINE items, so the item count is unchanged.)
        let a = FIXTURE;
        let b = format!("\n\n{a}");

        let map_a = AstIdMap::from_root(&tyrano_syntax::parse(a).syntax());
        let map_b = AstIdMap::from_root(&tyrano_syntax::parse(&b).syntax());

        assert_eq!(map_a.len(), map_b.len(), "leading blank lines changed len");

        for i in 0..map_a.len() {
            let id = ErasedAstId::new(i as u32);
            let pa = map_a.ptr(id);
            let pb = map_b.ptr(id);
            assert_eq!(pa.kind(), pb.kind(), "kind mismatch at id {i}");
            // Ranges differ because everything shifted by the 2 inserted bytes.
            assert_ne!(pa.text_range(), pb.text_range(), "range should shift at id {i}");
        }
    }

    #[test]
    fn ast_id_typed_roundtrip() {
        let parse = tyrano_syntax::parse(FIXTURE);
        let root = parse.syntax();
        let map = AstIdMap::from_root(&root);

        let label = root
            .descendants()
            .find_map(LabelLine::cast)
            .expect("fixture has a LABEL_LINE");

        let id: AstId<LabelLine> = map.ast_id(&label);
        let erased = id.erased();
        let resolved = map
            .resolve(erased, &root)
            .expect("id must resolve in its own tree");
        let back = LabelLine::cast(resolved).expect("resolved node casts back to LabelLine");
        assert_eq!(back.syntax().text_range(), label.syntax().text_range());
    }

    #[test]
    fn ast_id_is_hashmap_key() {
        // Compiling (and running) proves `AstId<N>` satisfies the Eq + Hash
        // bounds required of a HashMap key.
        let parse = tyrano_syntax::parse(FIXTURE);
        let root = parse.syntax();
        let map = AstIdMap::from_root(&root);

        let label = root.descendants().find_map(LabelLine::cast).unwrap();
        let id = map.ast_id(&label);

        let mut table: HashMap<AstId<LabelLine>, &str> = HashMap::new();
        table.insert(id, "start");
        assert_eq!(table.get(&id), Some(&"start"));
    }
}
