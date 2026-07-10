//! The immutable, position-independent **green tree**.
//!
//! The syntax stack uses a Roslyn / rust-analyzer style *red-green* split:
//!
//! - **Green** (this module) is the durable half. A green node stores only
//!   kinds, text, and *lengths* — never absolute offsets. Because a subtree
//!   carries no knowledge of where it sits in a document, the exact same
//!   `Arc`-shared green node can appear in many trees at once (incremental
//!   reparses reuse untouched subtrees) and be compared purely by content.
//! - **Red** (see [`crate::red`]) is the ephemeral half: lightweight cursors
//!   layered over a green tree that add absolute offsets, parent links, and
//!   traversal. Red cursors are cheap to create and throw away; the green
//!   tree they point at is the single source of truth for text.
//!
//! Everything here is `Send + Sync`: sharing is via [`Arc`], and there is no
//! interior mutability in any published type.
//!
//! # Full fidelity
//!
//! Concatenating every token's leading trivia, text, and trailing trivia in
//! tree order reproduces the original source byte-for-byte, including
//! whitespace, a BOM, escape backslashes, and invalid bytes. [`GreenNode::to_source`]
//! performs exactly that round-trip.

use std::collections::HashMap;
use std::hash::{Hash, Hasher};
use std::sync::Arc;

use crate::kind::SyntaxKind;
use crate::text::TextSize;

// ======================================================================
// Deterministic content hashing
// ======================================================================

/// Multiplier for the FxHash-style mixing step.
const FX_SEED: u64 = 0x51_7c_c1_b7_27_22_0a_95;

/// A small, allocation-free, **deterministic** hasher.
///
/// Unlike [`std::collections::hash_map::RandomState`], this seeds identically
/// on every run, so two structurally identical trees built independently
/// always produce the same [`GreenNode::content_hash`]. It is *not* a
/// cryptographic hash and its exact output is not a stability guarantee across
/// crate versions — it only needs to be consistent within a single program.
#[derive(Clone)]
struct FxHasher {
    hash: u64,
}

impl FxHasher {
    #[inline]
    fn new() -> FxHasher {
        FxHasher { hash: 0 }
    }

    #[inline]
    fn add_word(&mut self, word: u64) {
        self.hash = (self.hash.rotate_left(5) ^ word).wrapping_mul(FX_SEED);
    }
}

impl Hasher for FxHasher {
    #[inline]
    fn finish(&self) -> u64 {
        self.hash
    }

    #[inline]
    fn write(&mut self, bytes: &[u8]) {
        for chunk in bytes.chunks(8) {
            let mut buf = [0u8; 8];
            buf[..chunk.len()].copy_from_slice(chunk);
            self.add_word(u64::from_le_bytes(buf));
        }
        // Fold in the length so that e.g. `"a" ++ "bc"` and `"ab" ++ "c"`
        // (fed as separate `write` calls) cannot collide trivially.
        self.add_word(bytes.len() as u64 ^ 0x9e37_79b9_7f4a_7c15);
    }

    #[inline]
    fn write_u64(&mut self, i: u64) {
        self.add_word(i);
    }
}

/// Structural hash of a token's content: kind, text, trivia, and the
/// missing flag. Two tokens hash equal iff they are content-equal.
fn hash_token(data: &GreenTokenData) -> u64 {
    let mut h = FxHasher::new();
    hash_token_into(&mut h, data);
    h.finish()
}

/// The tag word marking the start of a token in a structural hash.
const TAG_TOKEN: u64 = 0xA5A5_0000_0000_0001;
/// The tag word marking the start of a node in a structural hash.
const TAG_NODE: u64 = 0xA5A5_0000_0000_0002;

fn hash_token_into(h: &mut FxHasher, data: &GreenTokenData) {
    h.write_u64(TAG_TOKEN);
    h.write_u64(data.kind.into_raw() as u64);
    h.write(data.text.as_bytes());
    h.write_u64(data.missing as u64);
    h.write_u64(data.leading.len() as u64);
    for t in data.leading.iter() {
        h.write_u64(t.kind().into_raw() as u64);
        h.write(t.text().as_bytes());
    }
    h.write_u64(data.trailing.len() as u64);
    for t in data.trailing.iter() {
        h.write_u64(t.kind().into_raw() as u64);
        h.write(t.text().as_bytes());
    }
}

/// Structural hash of a node from its kind and its children's hashes.
fn hash_node(kind: SyntaxKind, children: &[GreenElement]) -> u64 {
    let mut h = FxHasher::new();
    h.write_u64(TAG_NODE);
    h.write_u64(kind.into_raw() as u64);
    h.write_u64(children.len() as u64);
    for child in children {
        match child {
            GreenElement::Node(n) => {
                h.write_u64(1);
                h.write_u64(n.content_hash());
            }
            GreenElement::Token(t) => {
                h.write_u64(2);
                hash_token_into(&mut h, &t.0);
            }
        }
    }
    h.finish()
}

// ======================================================================
// GreenTrivia
// ======================================================================

/// A single piece of trivia (whitespace or a BOM) attached to a token.
///
/// Trivia never stands alone in the tree: it rides on a [`GreenToken`] as a
/// leading or trailing piece. The text is `Arc`-shared so that the builder's
/// interning cache can hand out one allocation for common pieces such as a
/// single space or `\n`.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct GreenTrivia {
    kind: SyntaxKind,
    text: Arc<str>,
}

impl GreenTrivia {
    /// Creates a trivia piece.
    ///
    /// `kind` must satisfy [`SyntaxKind::is_trivia`] (checked in debug builds).
    pub fn new(kind: SyntaxKind, text: &str) -> GreenTrivia {
        debug_assert!(kind.is_trivia(), "{kind} is not a trivia kind");
        GreenTrivia {
            kind,
            text: Arc::from(text),
        }
    }

    /// The trivia kind.
    #[inline]
    pub fn kind(&self) -> SyntaxKind {
        self.kind
    }

    /// The trivia's source text.
    #[inline]
    pub fn text(&self) -> &str {
        &self.text
    }

    /// The byte length of the trivia text.
    #[inline]
    pub fn text_len(&self) -> TextSize {
        TextSize::of(&self.text)
    }

    /// Test-only accessor for the shared text allocation, used to assert that
    /// interning hands out one `Arc` for repeated `(kind, text)` pairs.
    #[cfg(test)]
    pub(crate) fn text_arc(&self) -> &Arc<str> {
        &self.text
    }
}

// ======================================================================
// GreenToken
// ======================================================================

#[derive(Debug)]
struct GreenTokenData {
    kind: SyntaxKind,
    text: Arc<str>,
    leading: Box<[GreenTrivia]>,
    trailing: Box<[GreenTrivia]>,
    missing: bool,
}

/// An immutable leaf of the tree: a token plus its attached trivia.
///
/// Cloning is cheap — it bumps an [`Arc`] refcount. Equality, ordering-free
/// comparison, and hashing are all by **content** (kind, text, trivia, and
/// the missing flag), never by pointer identity, so tokens produced by two
/// independent parses compare equal when they spell the same thing.
#[derive(Debug, Clone)]
pub struct GreenToken(Arc<GreenTokenData>);

impl GreenToken {
    /// Creates a token with the given leading and trailing trivia.
    ///
    /// `kind` must satisfy [`SyntaxKind::is_token`] (checked in debug builds).
    pub fn new(
        kind: SyntaxKind,
        text: &str,
        leading: Vec<GreenTrivia>,
        trailing: Vec<GreenTrivia>,
    ) -> GreenToken {
        debug_assert!(kind.is_token(), "{kind} is not a token kind");
        GreenToken(Arc::new(GreenTokenData {
            kind,
            text: Arc::from(text),
            leading: leading.into_boxed_slice(),
            trailing: trailing.into_boxed_slice(),
            missing: false,
        }))
    }

    /// Creates a **missing** token: a zero-width placeholder the parser emits
    /// where a required token was absent. It has empty text, no trivia, and
    /// renders as `""`, so it never disturbs offsets.
    pub fn missing(kind: SyntaxKind) -> GreenToken {
        debug_assert!(kind.is_token(), "{kind} is not a token kind");
        GreenToken(Arc::new(GreenTokenData {
            kind,
            text: Arc::from(""),
            leading: Box::from([]),
            trailing: Box::from([]),
            missing: true,
        }))
    }

    /// The token kind.
    #[inline]
    pub fn kind(&self) -> SyntaxKind {
        self.0.kind
    }

    /// The token's own text, without any trivia.
    #[inline]
    pub fn text(&self) -> &str {
        &self.0.text
    }

    /// Whether this is a synthesized missing token.
    #[inline]
    pub fn is_missing(&self) -> bool {
        self.0.missing
    }

    /// The leading trivia pieces, in source order.
    #[inline]
    pub fn leading(&self) -> &[GreenTrivia] {
        &self.0.leading
    }

    /// The trailing trivia pieces, in source order.
    #[inline]
    pub fn trailing(&self) -> &[GreenTrivia] {
        &self.0.trailing
    }

    /// The byte length of the token text only (no trivia).
    #[inline]
    pub fn text_len(&self) -> TextSize {
        TextSize::of(&self.0.text)
    }

    /// The total byte length of leading trivia + text + trailing trivia.
    pub fn full_len(&self) -> TextSize {
        let leading: TextSize = self.0.leading.iter().map(|t| t.text_len()).sum();
        let trailing: TextSize = self.0.trailing.iter().map(|t| t.text_len()).sum();
        leading + self.text_len() + trailing
    }

    /// The combined byte length of the leading trivia.
    pub(crate) fn leading_len(&self) -> TextSize {
        self.0.leading.iter().map(|t| t.text_len()).sum()
    }

    /// Appends this token's full source (leading trivia, text, trailing
    /// trivia) to `out`, preserving every byte.
    pub fn write_source(&self, out: &mut String) {
        for t in self.0.leading.iter() {
            out.push_str(t.text());
        }
        out.push_str(&self.0.text);
        for t in self.0.trailing.iter() {
            out.push_str(t.text());
        }
    }

    /// Pointer identity check (same underlying allocation). Test-only: used
    /// to assert the builder's trivia-less token interning.
    #[cfg(test)]
    pub(crate) fn ptr_eq(a: &GreenToken, b: &GreenToken) -> bool {
        Arc::ptr_eq(&a.0, &b.0)
    }
}

impl PartialEq for GreenToken {
    fn eq(&self, other: &GreenToken) -> bool {
        Arc::ptr_eq(&self.0, &other.0)
            || (self.0.kind == other.0.kind
                && self.0.missing == other.0.missing
                && self.0.text == other.0.text
                && self.0.leading == other.0.leading
                && self.0.trailing == other.0.trailing)
    }
}

impl Eq for GreenToken {}

impl Hash for GreenToken {
    fn hash<H: Hasher>(&self, state: &mut H) {
        state.write_u64(hash_token(&self.0));
    }
}

// ======================================================================
// GreenElement
// ======================================================================

/// Either a node or a token: the child type held by [`GreenNode`].
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum GreenElement {
    /// An interior node.
    Node(GreenNode),
    /// A leaf token.
    Token(GreenToken),
}

impl GreenElement {
    /// The kind of the wrapped node or token.
    #[inline]
    pub fn kind(&self) -> SyntaxKind {
        match self {
            GreenElement::Node(n) => n.kind(),
            GreenElement::Token(t) => t.kind(),
        }
    }

    /// The full byte length, trivia included.
    #[inline]
    pub fn full_len(&self) -> TextSize {
        match self {
            GreenElement::Node(n) => n.full_len(),
            GreenElement::Token(t) => t.full_len(),
        }
    }

    fn write_source(&self, out: &mut String) {
        match self {
            GreenElement::Node(n) => n.write_source(out),
            GreenElement::Token(t) => t.write_source(out),
        }
    }
}

impl From<GreenNode> for GreenElement {
    #[inline]
    fn from(node: GreenNode) -> GreenElement {
        GreenElement::Node(node)
    }
}

impl From<GreenToken> for GreenElement {
    #[inline]
    fn from(token: GreenToken) -> GreenElement {
        GreenElement::Token(token)
    }
}

// ======================================================================
// GreenNode
// ======================================================================

#[derive(Debug)]
struct GreenNodeData {
    kind: SyntaxKind,
    full_len: TextSize,
    content_hash: u64,
    children: Box<[GreenElement]>,
}

/// An immutable interior node of the tree.
///
/// A green node stores its kind, its total byte length (trivia included), a
/// precomputed structural hash, and its children — but **no absolute
/// offset**. That position independence is what lets one `Arc`-shared node be
/// reused across edits and compared to another purely by content.
///
/// Cloning is cheap (an [`Arc`] bump). Equality first checks the cached
/// [`content_hash`](GreenNode::content_hash) as a fast reject, then falls back
/// to a deep structural comparison, so two independently built but identical
/// trees compare equal.
#[derive(Debug, Clone)]
pub struct GreenNode(Arc<GreenNodeData>);

impl GreenNode {
    /// Builds a node of `kind` from `children`, computing its full length
    /// (the sum of the children's full lengths) and its structural hash.
    ///
    /// `kind` must satisfy [`SyntaxKind::is_node`] (checked in debug builds).
    pub fn new(kind: SyntaxKind, children: Vec<GreenElement>) -> GreenNode {
        debug_assert!(kind.is_node(), "{kind} is not a node kind");
        let full_len: TextSize = children.iter().map(|c| c.full_len()).sum();
        let content_hash = hash_node(kind, &children);
        GreenNode(Arc::new(GreenNodeData {
            kind,
            full_len,
            content_hash,
            children: children.into_boxed_slice(),
        }))
    }

    /// The node kind.
    #[inline]
    pub fn kind(&self) -> SyntaxKind {
        self.0.kind
    }

    /// The total byte length of the subtree, trivia included.
    #[inline]
    pub fn full_len(&self) -> TextSize {
        self.0.full_len
    }

    /// The node's direct children, in source order.
    #[inline]
    pub fn children(&self) -> &[GreenElement] {
        &self.0.children
    }

    /// The number of direct children.
    #[inline]
    pub fn child_count(&self) -> usize {
        self.0.children.len()
    }

    /// The precomputed structural hash of this subtree.
    ///
    /// Deterministic: two structurally identical trees, however they were
    /// built, share the same value.
    #[inline]
    pub fn content_hash(&self) -> u64 {
        self.0.content_hash
    }

    /// The full round-trip source text of this subtree.
    pub fn to_source(&self) -> String {
        let mut out = String::with_capacity(self.full_len().to_usize());
        self.write_source(&mut out);
        out
    }

    /// Appends this subtree's full source text to `out`.
    pub fn write_source(&self, out: &mut String) {
        for child in self.0.children.iter() {
            child.write_source(out);
        }
    }

    /// Pointer identity check: whether `a` and `b` are the very same shared
    /// node (as happens after a builder splice reuses a subtree).
    #[inline]
    pub fn ptr_eq(a: &GreenNode, b: &GreenNode) -> bool {
        Arc::ptr_eq(&a.0, &b.0)
    }
}

impl PartialEq for GreenNode {
    fn eq(&self, other: &GreenNode) -> bool {
        Arc::ptr_eq(&self.0, &other.0)
            || (self.0.content_hash == other.0.content_hash
                && self.0.kind == other.0.kind
                && self.0.full_len == other.0.full_len
                && self.0.children == other.0.children)
    }
}

impl Eq for GreenNode {}

impl Hash for GreenNode {
    fn hash<H: Hasher>(&self, state: &mut H) {
        // Consistent with `Eq`: equal nodes share a `content_hash`.
        state.write_u64(self.0.content_hash);
    }
}

// ======================================================================
// GreenBuilder
// ======================================================================

/// A marker for a position in the builder's child stream, taken with
/// [`GreenBuilder::checkpoint`] and later handed to
/// [`GreenBuilder::start_node_at`] to wrap everything emitted since then.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Checkpoint(usize);

/// A stack-based builder for green trees, with interning caches.
///
/// The builder keeps a flat buffer of the children emitted so far plus a
/// stack of open parents. [`start_node`](GreenBuilder::start_node) pushes a
/// parent; [`finish_node`](GreenBuilder::finish_node) pops it, draining the
/// children emitted in between into a fresh [`GreenNode`].
///
/// Two interning caches keep common allocations shared: one for trivia
/// pieces (via [`trivia`](GreenBuilder::trivia)) and one for *trivia-less*
/// tokens keyed by `(kind, text)`, so that ubiquitous tokens like `]`, `=`,
/// or a lone `\n` reuse a single `Arc`. Node-level hash-consing is
/// intentionally not done.
pub struct GreenBuilder {
    /// `(kind, index-into-children-where-this-node-began)` for each open node.
    parents: Vec<(SyntaxKind, usize)>,
    /// Flat buffer of emitted children not yet folded into a parent.
    children: Vec<GreenElement>,
    /// Interned trivia pieces keyed by `(kind, text)`.
    trivia_cache: HashMap<(SyntaxKind, Box<str>), GreenTrivia>,
    /// Interned trivia-less tokens keyed by `(kind, text)`.
    token_cache: HashMap<(SyntaxKind, Box<str>), GreenToken>,
}

impl GreenBuilder {
    /// Creates an empty builder.
    pub fn new() -> GreenBuilder {
        GreenBuilder {
            parents: Vec::new(),
            children: Vec::new(),
            trivia_cache: HashMap::new(),
            token_cache: HashMap::new(),
        }
    }

    /// Opens a new node of `kind`; its children are everything emitted until
    /// the matching [`finish_node`](GreenBuilder::finish_node).
    pub fn start_node(&mut self, kind: SyntaxKind) {
        self.parents.push((kind, self.children.len()));
    }

    /// Closes the most recently opened node, folding the children emitted
    /// since it was opened into a fresh [`GreenNode`].
    ///
    /// # Panics
    /// Panics if there is no open node.
    pub fn finish_node(&mut self) {
        let (kind, first_child) = self
            .parents
            .pop()
            .expect("finish_node called with no open node");
        let children: Vec<GreenElement> = self.children.split_off(first_child);
        let node = GreenNode::new(kind, children);
        self.children.push(GreenElement::Node(node));
    }

    /// Records the current position so a later
    /// [`start_node_at`](GreenBuilder::start_node_at) can retroactively wrap
    /// everything emitted after this point.
    pub fn checkpoint(&self) -> Checkpoint {
        Checkpoint(self.children.len())
    }

    /// Wraps everything emitted since `cp` into a new node of `kind`
    /// (rust-analyzer's `start_node_at`). Used for retroactive promotion,
    /// e.g. deciding a run of tokens was a `TEXT_LINE` only after seeing it.
    ///
    /// # Panics
    /// Panics if `cp` is no longer valid — either children have since been
    /// folded past it (a `finish_node` fired early) or an unmatched
    /// `start_node_at` moved the frontier ahead of it.
    pub fn start_node_at(&mut self, cp: Checkpoint, kind: SyntaxKind) {
        let Checkpoint(cp) = cp;
        assert!(
            cp <= self.children.len(),
            "checkpoint no longer valid: was finish_node called early?"
        );
        if let Some(&(_, first_child)) = self.parents.last() {
            assert!(
                cp >= first_child,
                "checkpoint no longer valid: it predates an open node"
            );
        }
        self.parents.push((kind, cp));
    }

    /// Emits a token of `kind` with the given trivia. Trivia-less tokens are
    /// interned by `(kind, text)` so repeated punctuation shares one `Arc`.
    pub fn token(
        &mut self,
        kind: SyntaxKind,
        text: &str,
        leading: Vec<GreenTrivia>,
        trailing: Vec<GreenTrivia>,
    ) {
        let token = if leading.is_empty() && trailing.is_empty() {
            self.intern_token(kind, text)
        } else {
            GreenToken::new(kind, text, leading, trailing)
        };
        self.children.push(GreenElement::Token(token));
    }

    /// Emits a missing (zero-width placeholder) token of `kind`.
    pub fn missing_token(&mut self, kind: SyntaxKind) {
        self.children
            .push(GreenElement::Token(GreenToken::missing(kind)));
    }

    /// Splices an existing green subtree straight into the current node,
    /// reusing its `Arc` (incremental reuse — no rebuild, no rehash).
    pub fn node(&mut self, node: GreenNode) {
        self.children.push(GreenElement::Node(node));
    }

    /// Returns an interned trivia piece for `(kind, text)`, sharing one
    /// allocation across every call with the same pair.
    pub fn trivia(&mut self, kind: SyntaxKind, text: &str) -> GreenTrivia {
        if let Some(existing) = self.trivia_cache.get(&(kind, Box::from(text))) {
            return existing.clone();
        }
        let trivia = GreenTrivia::new(kind, text);
        self.trivia_cache
            .insert((kind, Box::from(text)), trivia.clone());
        trivia
    }

    /// Interns a trivia-less token by `(kind, text)`.
    fn intern_token(&mut self, kind: SyntaxKind, text: &str) -> GreenToken {
        if let Some(existing) = self.token_cache.get(&(kind, Box::from(text))) {
            return existing.clone();
        }
        let token = GreenToken::new(kind, text, Vec::new(), Vec::new());
        self.token_cache
            .insert((kind, Box::from(text)), token.clone());
        token
    }

    /// Finishes building and returns the single root node.
    ///
    /// # Panics
    /// Panics if the node stack is unbalanced — some `start_node` was never
    /// matched by a `finish_node`, or nothing (or more than one root) was
    /// produced.
    pub fn finish(mut self) -> GreenNode {
        assert!(
            self.parents.is_empty(),
            "unbalanced builder: {} node(s) left open",
            self.parents.len()
        );
        assert_eq!(
            self.children.len(),
            1,
            "unbalanced builder: expected exactly one root element"
        );
        match self.children.pop().expect("checked len == 1") {
            GreenElement::Node(node) => node,
            GreenElement::Token(_) => {
                panic!("unbalanced builder: root element is a token, not a node")
            }
        }
    }
}

impl Default for GreenBuilder {
    fn default() -> GreenBuilder {
        GreenBuilder::new()
    }
}

// ======================================================================
// Tests
// ======================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use crate::kind::SyntaxKind as K;

    fn ws(text: &str) -> GreenTrivia {
        GreenTrivia::new(K::WHITESPACE, text)
    }

    fn tok(kind: SyntaxKind, text: &str) -> GreenElement {
        GreenToken::new(kind, text, Vec::new(), Vec::new()).into()
    }

    /// Builds `*start\n` as a LABEL_LINE and a TEXT_LINE `  hi` whose TEXT
    /// token carries leading WHITESPACE trivia, under a SCENARIO root.
    fn sample_scenario() -> GreenNode {
        let star = tok(K::STAR, "*");
        let name = GreenNode::new(K::LABEL_NAME, vec![tok(K::TEXT, "start")]);
        let nl = tok(K::NEWLINE, "\n");
        let label = GreenNode::new(
            K::LABEL_LINE,
            vec![star, GreenElement::Node(name), nl],
        );

        let text_tok =
            GreenToken::new(K::TEXT, "hi", vec![ws("  ")], Vec::new());
        let text_line =
            GreenNode::new(K::TEXT_LINE, vec![GreenElement::Token(text_tok)]);

        GreenNode::new(
            K::SCENARIO,
            vec![
                GreenElement::Node(label),
                GreenElement::Node(text_line),
                GreenToken::missing(K::EOF).into(),
            ],
        )
    }

    #[test]
    fn trivia_basics() {
        let t = GreenTrivia::new(K::WHITESPACE, "   ");
        assert_eq!(t.kind(), K::WHITESPACE);
        assert_eq!(t.text(), "   ");
        assert_eq!(t.text_len(), TextSize::new(3));
    }

    #[test]
    fn token_lengths_and_source() {
        let t = GreenToken::new(K::TEXT, "hi", vec![ws("  ")], vec![ws(" ")]);
        assert_eq!(t.text(), "hi");
        assert_eq!(t.text_len(), TextSize::new(2));
        assert_eq!(t.full_len(), TextSize::new(5)); // "  " + "hi" + " "
        assert_eq!(t.leading_len(), TextSize::new(2));
        let mut s = String::new();
        t.write_source(&mut s);
        assert_eq!(s, "  hi ");
    }

    #[test]
    fn missing_token_is_empty() {
        let t = GreenToken::missing(K::EOF);
        assert!(t.is_missing());
        assert_eq!(t.text(), "");
        assert_eq!(t.full_len(), TextSize::new(0));
        let mut s = String::new();
        t.write_source(&mut s);
        assert_eq!(s, "");
    }

    #[test]
    fn full_len_invariant_and_roundtrip() {
        let root = sample_scenario();
        // Σ children.full_len() == node.full_len().
        let sum: TextSize = root.children().iter().map(|c| c.full_len()).sum();
        assert_eq!(sum, root.full_len());
        // to_source reproduces the input exactly, missing token contributes "".
        assert_eq!(root.to_source(), "*start\n  hi");
        assert_eq!(root.full_len(), TextSize::new(11));
        assert_eq!(root.child_count(), 3);
    }

    #[test]
    fn roundtrip_preserves_bom() {
        let bom = GreenTrivia::new(K::BOM, "\u{feff}");
        let text = GreenToken::new(K::TEXT, "hi", vec![bom], Vec::new());
        let line = GreenNode::new(K::TEXT_LINE, vec![GreenElement::Token(text)]);
        let root = GreenNode::new(K::SCENARIO, vec![GreenElement::Node(line)]);
        assert_eq!(root.to_source(), "\u{feff}hi");
        // The BOM is 3 bytes.
        assert_eq!(root.full_len(), TextSize::new(5));
    }

    #[test]
    fn element_from_and_kind() {
        let n: GreenElement = GreenNode::new(K::ERROR, vec![]).into();
        assert!(matches!(n, GreenElement::Node(_)));
        assert_eq!(n.kind(), K::ERROR);
        let t: GreenElement = GreenToken::new(K::STAR, "*", vec![], vec![]).into();
        assert_eq!(t.kind(), K::STAR);
        assert_eq!(t.full_len(), TextSize::new(1));
    }

    #[test]
    fn structural_equality_and_hash() {
        // Two independently built identical trees compare equal and hash equal.
        let a = sample_scenario();
        let b = sample_scenario();
        assert!(!GreenNode::ptr_eq(&a, &b), "distinct allocations expected");
        assert_eq!(a, b);
        assert_eq!(a.content_hash(), b.content_hash());

        // A different text yields a different hash (probabilistically) and
        // an unequal tree.
        let star = tok(K::STAR, "*");
        let name = GreenNode::new(K::LABEL_NAME, vec![tok(K::TEXT, "other")]);
        let nl = tok(K::NEWLINE, "\n");
        let label = GreenNode::new(K::LABEL_LINE, vec![star, name.into(), nl]);
        let different = GreenNode::new(K::SCENARIO, vec![label.into()]);
        assert_ne!(a, different);
        assert_ne!(a.content_hash(), different.content_hash());
    }

    #[test]
    fn token_equality_by_content() {
        let a = GreenToken::new(K::EQ, "=", vec![], vec![]);
        let b = GreenToken::new(K::EQ, "=", vec![], vec![]);
        assert_eq!(a, b);
        // Missing flag participates in equality.
        let m = GreenToken::missing(K::EQ);
        let empty = GreenToken::new(K::EQ, "", vec![], vec![]);
        assert_ne!(m, empty);
    }

    #[test]
    fn builder_builds_scenario() {
        let mut b = GreenBuilder::new();
        b.start_node(K::SCENARIO);
        b.start_node(K::LABEL_LINE);
        b.token(K::STAR, "*", vec![], vec![]);
        b.token(K::TEXT, "start", vec![], vec![]);
        b.token(K::NEWLINE, "\n", vec![], vec![]);
        b.finish_node();
        b.finish_node();
        let root = b.finish();
        assert_eq!(root.kind(), K::SCENARIO);
        assert_eq!(root.to_source(), "*start\n");
    }

    #[test]
    fn builder_default_matches_new() {
        let mut b = GreenBuilder::default();
        b.start_node(K::SCENARIO);
        b.token(K::TEXT, "x", vec![], vec![]);
        b.finish_node();
        assert_eq!(b.finish().to_source(), "x");
    }

    #[test]
    fn checkpoint_wraps_retroactively() {
        // Build `a [b] c`, then retroactively wrap the middle `b` in a node.
        let mut b = GreenBuilder::new();
        b.start_node(K::SCENARIO);
        b.token(K::TEXT, "a", vec![], vec![]);
        let cp = b.checkpoint();
        b.token(K::TEXT, "b", vec![], vec![]);
        b.start_node_at(cp, K::TEXT_LINE);
        b.finish_node();
        b.token(K::TEXT, "c", vec![], vec![]);
        b.finish_node();
        let root = b.finish();
        // scenario has: TEXT "a", TEXT_LINE(TEXT "b"), TEXT "c".
        assert_eq!(root.child_count(), 3);
        assert!(matches!(root.children()[0], GreenElement::Token(_)));
        match &root.children()[1] {
            GreenElement::Node(n) => {
                assert_eq!(n.kind(), K::TEXT_LINE);
                assert_eq!(n.to_source(), "b");
            }
            _ => panic!("expected wrapped TEXT_LINE"),
        }
        assert_eq!(root.to_source(), "abc");
    }

    #[test]
    fn builder_splice_reuses_subtree() {
        // A pre-built subtree spliced via `node()` keeps its identity.
        let original = GreenNode::new(
            K::INLINE_TAG,
            vec![
                tok(K::L_BRACKET, "["),
                tok(K::IDENT, "l"),
                tok(K::R_BRACKET, "]"),
            ],
        );
        let mut b = GreenBuilder::new();
        b.start_node(K::SCENARIO);
        b.start_node(K::TEXT_LINE);
        b.node(original.clone());
        b.finish_node();
        b.finish_node();
        let root = b.finish();
        let text_line = match &root.children()[0] {
            GreenElement::Node(n) => n,
            _ => panic!(),
        };
        let spliced = match &text_line.children()[0] {
            GreenElement::Node(n) => n,
            _ => panic!(),
        };
        assert!(GreenNode::ptr_eq(&original, spliced), "subtree must be reused");
    }

    #[test]
    fn interning_shares_trivia_allocation() {
        let mut b = GreenBuilder::new();
        let a = b.trivia(K::WHITESPACE, " ");
        let c = b.trivia(K::WHITESPACE, " ");
        assert!(
            Arc::ptr_eq(a.text_arc(), c.text_arc()),
            "identical trivia should share one Arc"
        );
        // A different text is a different allocation.
        let d = b.trivia(K::WHITESPACE, "  ");
        assert!(!Arc::ptr_eq(a.text_arc(), d.text_arc()));
    }

    #[test]
    fn interning_shares_trivialess_tokens() {
        let mut b = GreenBuilder::new();
        b.start_node(K::SCENARIO);
        b.token(K::R_BRACKET, "]", vec![], vec![]);
        b.token(K::R_BRACKET, "]", vec![], vec![]);
        b.finish_node();
        let root = b.finish();
        let t0 = match &root.children()[0] {
            GreenElement::Token(t) => t.clone(),
            _ => panic!(),
        };
        let t1 = match &root.children()[1] {
            GreenElement::Token(t) => t.clone(),
            _ => panic!(),
        };
        assert!(GreenToken::ptr_eq(&t0, &t1), "trivia-less tokens should intern");
    }

    #[test]
    #[should_panic(expected = "unbalanced")]
    fn unbalanced_builder_panics() {
        let mut b = GreenBuilder::new();
        b.start_node(K::SCENARIO);
        b.token(K::TEXT, "x", vec![], vec![]);
        // Missing finish_node for SCENARIO.
        let _ = b.finish();
    }

    #[test]
    fn send_sync() {
        fn assert_send_sync<T: Send + Sync>() {}
        assert_send_sync::<GreenNode>();
        assert_send_sync::<GreenToken>();
        assert_send_sync::<GreenTrivia>();
        assert_send_sync::<GreenElement>();
    }
}
