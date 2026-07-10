//! The **red cursor API**: ephemeral, offset-aware views over a green tree.
//!
//! The green tree ([`crate::green`]) is position-independent and immutable —
//! it knows kinds, text, and lengths, but not *where* any node sits. The red
//! layer supplies what the green layer deliberately omits:
//!
//! - **absolute byte offsets**, computed lazily as you descend;
//! - **parent links**, so you can walk back up toward the root;
//! - **traversal**: children, siblings, ancestors, descendants, and
//!   document-order token iteration.
//!
//! A [`SyntaxNode`] / [`SyntaxToken`] is a thin cursor: it holds an [`Arc`] to
//! its own materialized data (green node + parent + offset + index), so
//! cloning is a cheap refcount bump and cursors are created and discarded
//! freely. Two cursors are equal when they denote the same green data at the
//! same offset and child index.
//!
//! Everything here is `Send + Sync`: sharing is via [`Arc`], with no interior
//! mutability.

use std::hash::{Hash, Hasher};
use std::sync::Arc;

use crate::green::{GreenElement, GreenNode, GreenToken};
use crate::kind::SyntaxKind;
use crate::text::{TextRange, TextSize};

// ======================================================================
// SyntaxNode
// ======================================================================

/// The materialized data behind a [`SyntaxNode`] cursor.
struct NodeData {
    /// Parent cursor, or `None` for the root.
    parent: Option<SyntaxNode>,
    /// The green node this cursor points at.
    green: GreenNode,
    /// Absolute byte offset of this node's full range (trivia included).
    offset: TextSize,
    /// Index of this node in its parent's `children_with_tokens`.
    index: usize,
}

/// A red cursor over an interior node: a green node plus its absolute
/// position and a link to its parent.
///
/// Cloning is cheap (an [`Arc`] bump). See the [module docs](self) for the
/// red-green split.
#[derive(Clone)]
pub struct SyntaxNode {
    data: Arc<NodeData>,
}

impl SyntaxNode {
    /// Creates the root cursor for `green` at offset 0 with no parent.
    pub fn new_root(green: GreenNode) -> SyntaxNode {
        SyntaxNode {
            data: Arc::new(NodeData {
                parent: None,
                green,
                offset: TextSize::new(0),
                index: 0,
            }),
        }
    }

    fn new_child(parent: SyntaxNode, green: GreenNode, offset: TextSize, index: usize) -> SyntaxNode {
        SyntaxNode {
            data: Arc::new(NodeData {
                parent: Some(parent),
                green,
                offset,
                index,
            }),
        }
    }

    /// The node kind.
    #[inline]
    pub fn kind(&self) -> SyntaxKind {
        self.data.green.kind()
    }

    /// The underlying green node.
    #[inline]
    pub fn green(&self) -> &GreenNode {
        &self.data.green
    }

    /// The absolute byte range covering **all** content of this subtree,
    /// including the leading trivia of its first token and the trailing
    /// trivia of its last.
    #[inline]
    pub fn text_range(&self) -> TextRange {
        TextRange::at(self.data.offset, self.data.green.full_len())
    }

    /// The absolute range with the first token's leading trivia and the last
    /// token's trailing trivia trimmed off. Equals [`text_range`](Self::text_range)
    /// for a subtree with no edge trivia; an empty node collapses to a
    /// zero-width range at its offset.
    pub fn trimmed_range(&self) -> TextRange {
        match (self.first_token(), self.last_token()) {
            (Some(first), Some(last)) => {
                TextRange::new(first.text_range().start(), last.text_range().end())
            }
            _ => TextRange::empty(self.data.offset),
        }
    }

    /// The parent node, or `None` at the root.
    #[inline]
    pub fn parent(&self) -> Option<SyntaxNode> {
        self.data.parent.clone()
    }

    /// This node's index in its parent's `children_with_tokens`.
    #[inline]
    pub fn index(&self) -> usize {
        self.data.index
    }

    /// The child nodes, in order (tokens skipped).
    pub fn children(&self) -> impl Iterator<Item = SyntaxNode> + '_ {
        self.children_with_tokens()
            .filter_map(|e| e.into_node())
    }

    /// The children and tokens, in order, each with its absolute offset.
    pub fn children_with_tokens(&self) -> impl Iterator<Item = SyntaxElement> + '_ {
        let parent = self.clone();
        let mut offset = self.data.offset;
        let mut out = Vec::with_capacity(self.data.green.child_count());
        for (index, child) in self.data.green.children().iter().enumerate() {
            let child_offset = offset;
            match child {
                GreenElement::Node(n) => out.push(SyntaxElement::Node(SyntaxNode::new_child(
                    parent.clone(),
                    n.clone(),
                    child_offset,
                    index,
                ))),
                GreenElement::Token(t) => out.push(SyntaxElement::Token(SyntaxToken::new(
                    parent.clone(),
                    t.clone(),
                    child_offset,
                    index,
                ))),
            }
            offset += child.full_len();
        }
        out.into_iter()
    }

    /// This node and all descendant nodes, in pre-order.
    pub fn descendants(&self) -> impl Iterator<Item = SyntaxNode> {
        fn go(node: &SyntaxNode, out: &mut Vec<SyntaxNode>) {
            out.push(node.clone());
            for child in node.children() {
                go(&child, out);
            }
        }
        let mut out = Vec::new();
        go(self, &mut out);
        out.into_iter()
    }

    /// This node and all descendant nodes and tokens, in pre-order.
    pub fn descendants_with_tokens(&self) -> impl Iterator<Item = SyntaxElement> {
        fn go(node: &SyntaxNode, out: &mut Vec<SyntaxElement>) {
            out.push(SyntaxElement::Node(node.clone()));
            for el in node.children_with_tokens() {
                match el {
                    SyntaxElement::Node(n) => go(&n, out),
                    SyntaxElement::Token(t) => out.push(SyntaxElement::Token(t)),
                }
            }
        }
        let mut out = Vec::new();
        go(self, &mut out);
        out.into_iter()
    }

    /// This node, then its parent, and so on up to the root.
    pub fn ancestors(&self) -> impl Iterator<Item = SyntaxNode> {
        let mut out = Vec::new();
        let mut cur = Some(self.clone());
        while let Some(node) = cur {
            cur = node.parent();
            out.push(node);
        }
        out.into_iter()
    }

    /// The first child node, if any.
    pub fn first_child(&self) -> Option<SyntaxNode> {
        self.children().next()
    }

    /// The last child node, if any.
    pub fn last_child(&self) -> Option<SyntaxNode> {
        self.children().last()
    }

    /// The next sibling that is a node.
    pub fn next_sibling(&self) -> Option<SyntaxNode> {
        let parent = self.parent()?;
        parent
            .children_with_tokens()
            .skip(self.index() + 1)
            .find_map(|e| e.into_node())
    }

    /// The previous sibling that is a node.
    pub fn prev_sibling(&self) -> Option<SyntaxNode> {
        let parent = self.parent()?;
        parent
            .children_with_tokens()
            .take(self.index())
            .filter_map(|e| e.into_node())
            .last()
    }

    /// The next sibling, node or token.
    pub fn next_sibling_or_token(&self) -> Option<SyntaxElement> {
        let parent = self.parent()?;
        parent.children_with_tokens().nth(self.index() + 1)
    }

    /// The previous sibling, node or token.
    pub fn prev_sibling_or_token(&self) -> Option<SyntaxElement> {
        let idx = self.index().checked_sub(1)?;
        let parent = self.parent()?;
        parent.children_with_tokens().nth(idx)
    }

    /// The leftmost descendant token.
    pub fn first_token(&self) -> Option<SyntaxToken> {
        self.descendants_with_tokens().find_map(|e| e.into_token())
    }

    /// The rightmost descendant token.
    pub fn last_token(&self) -> Option<SyntaxToken> {
        self.descendants_with_tokens()
            .filter_map(|e| e.into_token())
            .last()
    }

    /// The full-fidelity source text of this subtree.
    pub fn text(&self) -> String {
        self.data.green.to_source()
    }

    /// The token whose full range (trivia included) contains `offset`.
    ///
    /// Every byte of the input belongs to exactly one token's full range, so
    /// a byte strictly inside a token — even inside its leading trivia —
    /// resolves to [`TokenAtOffset::Single`] of that token. At an exact
    /// boundary between two adjacent tokens the result is
    /// [`TokenAtOffset::Between`]; at the very start or the very end of the
    /// text it is `Single` of the sole neighbouring token. Zero-width tokens
    /// (missing tokens and an empty EOF) are skipped.
    pub fn token_at_offset(&self, offset: TextSize) -> TokenAtOffset {
        let tokens: Vec<SyntaxToken> = self
            .descendants_with_tokens()
            .filter_map(|e| e.into_token())
            .filter(|t| !t.full_range().is_empty())
            .collect();

        let mut left = None;
        let mut right = None;
        for t in &tokens {
            let fr = t.full_range();
            if fr.start() < offset && offset < fr.end() {
                return TokenAtOffset::Single(t.clone());
            }
            if fr.end() == offset {
                left = Some(t.clone());
            }
            if fr.start() == offset {
                right = Some(t.clone());
            }
        }
        match (left, right) {
            (Some(l), Some(r)) => TokenAtOffset::Between(l, r),
            (Some(l), None) => TokenAtOffset::Single(l),
            (None, Some(r)) => TokenAtOffset::Single(r),
            (None, None) => TokenAtOffset::None,
        }
    }

    /// The smallest element whose [`text_range`](SyntaxElement::text_range)
    /// contains `range`. A range that spans two siblings resolves to their
    /// common parent.
    pub fn covering_element(&self, range: TextRange) -> SyntaxElement {
        let mut node = self.clone();
        loop {
            let child = node
                .children_with_tokens()
                .find(|el| el.text_range().contains_range(range));
            match child {
                Some(SyntaxElement::Node(n)) => node = n,
                Some(SyntaxElement::Token(t)) => return SyntaxElement::Token(t),
                None => return SyntaxElement::Node(node),
            }
        }
    }

    /// The smallest node whose range contains `offset` (this node itself when
    /// the offset lands directly on leaf-level tokens).
    pub fn find_node_at_offset(&self, offset: TextSize) -> Option<SyntaxNode> {
        if !self.text_range().contains_inclusive(offset) {
            return None;
        }
        let mut node = self.clone();
        'descend: loop {
            let children: Vec<SyntaxNode> = node.children().collect();
            for child in children {
                if child.text_range().contains_inclusive(offset) {
                    node = child;
                    continue 'descend;
                }
            }
            return Some(node);
        }
    }

    /// The root of the tree this cursor belongs to.
    fn root(&self) -> SyntaxNode {
        let mut node = self.clone();
        while let Some(parent) = node.parent() {
            node = parent;
        }
        node
    }
}

impl PartialEq for SyntaxNode {
    fn eq(&self, other: &SyntaxNode) -> bool {
        GreenNode::ptr_eq(&self.data.green, &other.data.green)
            && self.data.offset == other.data.offset
            && self.data.index == other.data.index
    }
}

impl Eq for SyntaxNode {}

impl Hash for SyntaxNode {
    fn hash<H: Hasher>(&self, state: &mut H) {
        // Consistent with `Eq`: ptr-equal greens share a content hash.
        state.write_u64(self.data.green.content_hash());
        self.data.offset.raw().hash(state);
        self.data.index.hash(state);
    }
}

impl std::fmt::Debug for SyntaxNode {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}@{:?}", self.kind(), self.text_range())
    }
}

impl std::fmt::Display for SyntaxNode {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.text())
    }
}

// ======================================================================
// SyntaxToken
// ======================================================================

/// A red cursor over a leaf token: a green token plus its absolute position
/// and a link to its parent node.
///
/// The token's [`text_range`](Self::text_range) covers its text only, while
/// [`full_range`](Self::full_range) also spans the leading and trailing
/// trivia. Cloning is cheap.
#[derive(Clone)]
pub struct SyntaxToken {
    parent: SyntaxNode,
    green: GreenToken,
    /// Absolute offset of the token's full range start (leading trivia first).
    offset: TextSize,
    index: usize,
}

impl SyntaxToken {
    fn new(parent: SyntaxNode, green: GreenToken, offset: TextSize, index: usize) -> SyntaxToken {
        SyntaxToken {
            parent,
            green,
            offset,
            index,
        }
    }

    /// The token kind.
    #[inline]
    pub fn kind(&self) -> SyntaxKind {
        self.green.kind()
    }

    /// The underlying green token.
    #[inline]
    pub fn green(&self) -> &GreenToken {
        &self.green
    }

    /// The token text only (no trivia).
    #[inline]
    pub fn text(&self) -> &str {
        self.green.text()
    }

    /// Whether this is a synthesized missing token.
    #[inline]
    pub fn is_missing(&self) -> bool {
        self.green.is_missing()
    }

    /// The absolute range of the token text only (leading/trailing trivia
    /// excluded).
    pub fn text_range(&self) -> TextRange {
        let start = self.offset + self.green.leading_len();
        TextRange::at(start, self.green.text_len())
    }

    /// The absolute range of the token including its leading and trailing
    /// trivia.
    pub fn full_range(&self) -> TextRange {
        TextRange::at(self.offset, self.green.full_len())
    }

    /// The parent node.
    #[inline]
    pub fn parent(&self) -> SyntaxNode {
        self.parent.clone()
    }

    /// This token's index in its parent's `children_with_tokens`.
    #[inline]
    pub fn index(&self) -> usize {
        self.index
    }

    /// The next sibling, node or token.
    pub fn next_sibling_or_token(&self) -> Option<SyntaxElement> {
        self.parent.children_with_tokens().nth(self.index + 1)
    }

    /// The previous sibling, node or token.
    pub fn prev_sibling_or_token(&self) -> Option<SyntaxElement> {
        let idx = self.index.checked_sub(1)?;
        self.parent.children_with_tokens().nth(idx)
    }

    /// The next token in whole-tree document order.
    pub fn next_token(&self) -> Option<SyntaxToken> {
        let root = self.parent.root();
        let mut iter = root
            .descendants_with_tokens()
            .filter_map(|e| e.into_token());
        for t in iter.by_ref() {
            if &t == self {
                return iter.next();
            }
        }
        None
    }

    /// The previous token in whole-tree document order.
    pub fn prev_token(&self) -> Option<SyntaxToken> {
        let root = self.parent.root();
        let tokens: Vec<SyntaxToken> = root
            .descendants_with_tokens()
            .filter_map(|e| e.into_token())
            .collect();
        let pos = tokens.iter().position(|t| t == self)?;
        pos.checked_sub(1).map(|i| tokens[i].clone())
    }

    /// The `(kind, absolute range)` of each leading trivia piece, in order.
    pub fn leading_trivia_ranges(&self) -> Vec<(SyntaxKind, TextRange)> {
        let mut out = Vec::with_capacity(self.green.leading().len());
        let mut offset = self.offset;
        for t in self.green.leading() {
            let range = TextRange::at(offset, t.text_len());
            out.push((t.kind(), range));
            offset += t.text_len();
        }
        out
    }

    /// The `(kind, absolute range)` of each trailing trivia piece, in order.
    pub fn trailing_trivia_ranges(&self) -> Vec<(SyntaxKind, TextRange)> {
        let mut out = Vec::with_capacity(self.green.trailing().len());
        // Trailing trivia begins right after the token text.
        let mut offset = self.offset + self.green.leading_len() + self.green.text_len();
        for t in self.green.trailing() {
            let range = TextRange::at(offset, t.text_len());
            out.push((t.kind(), range));
            offset += t.text_len();
        }
        out
    }
}

impl PartialEq for SyntaxToken {
    fn eq(&self, other: &SyntaxToken) -> bool {
        self.green == other.green && self.offset == other.offset && self.index == other.index
    }
}

impl Eq for SyntaxToken {}

impl Hash for SyntaxToken {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.green.hash(state);
        self.offset.raw().hash(state);
        self.index.hash(state);
    }
}

impl std::fmt::Debug for SyntaxToken {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}@{:?} {:?}", self.kind(), self.text_range(), self.text())
    }
}

impl std::fmt::Display for SyntaxToken {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.text())
    }
}

// ======================================================================
// SyntaxElement
// ======================================================================

/// Either a node or a token cursor.
#[derive(Clone, PartialEq, Eq, Hash)]
pub enum SyntaxElement {
    /// An interior node.
    Node(SyntaxNode),
    /// A leaf token.
    Token(SyntaxToken),
}

impl SyntaxElement {
    /// The kind of the wrapped node or token.
    pub fn kind(&self) -> SyntaxKind {
        match self {
            SyntaxElement::Node(n) => n.kind(),
            SyntaxElement::Token(t) => t.kind(),
        }
    }

    /// The element's text range. For a node this is its full range (trivia
    /// included); for a token it is the token text only.
    pub fn text_range(&self) -> TextRange {
        match self {
            SyntaxElement::Node(n) => n.text_range(),
            SyntaxElement::Token(t) => t.text_range(),
        }
    }

    /// The element's full range including trivia. For a node this equals
    /// [`text_range`](Self::text_range); for a token it spans its trivia too.
    pub fn full_range(&self) -> TextRange {
        match self {
            SyntaxElement::Node(n) => n.text_range(),
            SyntaxElement::Token(t) => t.full_range(),
        }
    }

    /// The parent node, or `None` for a root node element.
    pub fn parent(&self) -> Option<SyntaxNode> {
        match self {
            SyntaxElement::Node(n) => n.parent(),
            SyntaxElement::Token(t) => Some(t.parent()),
        }
    }

    /// The element's index in its parent's `children_with_tokens`.
    pub fn index(&self) -> usize {
        match self {
            SyntaxElement::Node(n) => n.index(),
            SyntaxElement::Token(t) => t.index(),
        }
    }

    /// Borrows the inner node, if this is a node.
    pub fn as_node(&self) -> Option<&SyntaxNode> {
        match self {
            SyntaxElement::Node(n) => Some(n),
            SyntaxElement::Token(_) => None,
        }
    }

    /// Borrows the inner token, if this is a token.
    pub fn as_token(&self) -> Option<&SyntaxToken> {
        match self {
            SyntaxElement::Token(t) => Some(t),
            SyntaxElement::Node(_) => None,
        }
    }

    /// Unwraps into the inner node, if this is a node.
    pub fn into_node(self) -> Option<SyntaxNode> {
        match self {
            SyntaxElement::Node(n) => Some(n),
            SyntaxElement::Token(_) => None,
        }
    }

    /// Unwraps into the inner token, if this is a token.
    pub fn into_token(self) -> Option<SyntaxToken> {
        match self {
            SyntaxElement::Token(t) => Some(t),
            SyntaxElement::Node(_) => None,
        }
    }
}

impl From<SyntaxNode> for SyntaxElement {
    fn from(node: SyntaxNode) -> SyntaxElement {
        SyntaxElement::Node(node)
    }
}

impl From<SyntaxToken> for SyntaxElement {
    fn from(token: SyntaxToken) -> SyntaxElement {
        SyntaxElement::Token(token)
    }
}

impl std::fmt::Debug for SyntaxElement {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            SyntaxElement::Node(n) => std::fmt::Debug::fmt(n, f),
            SyntaxElement::Token(t) => std::fmt::Debug::fmt(t, f),
        }
    }
}

// ======================================================================
// TokenAtOffset
// ======================================================================

/// The result of [`SyntaxNode::token_at_offset`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TokenAtOffset {
    /// No token at the offset (an empty tree).
    None,
    /// The offset lies strictly inside a single token, or at the very edge
    /// of the text with only one neighbour.
    Single(SyntaxToken),
    /// The offset sits exactly on the boundary between two adjacent tokens.
    Between(SyntaxToken, SyntaxToken),
}

impl TokenAtOffset {
    /// Prefers the token to the right of a boundary.
    pub fn right_biased(self) -> Option<SyntaxToken> {
        match self {
            TokenAtOffset::None => None,
            TokenAtOffset::Single(t) => Some(t),
            TokenAtOffset::Between(_, right) => Some(right),
        }
    }

    /// Prefers the token to the left of a boundary.
    pub fn left_biased(self) -> Option<SyntaxToken> {
        match self {
            TokenAtOffset::None => None,
            TokenAtOffset::Single(t) => Some(t),
            TokenAtOffset::Between(left, _) => Some(left),
        }
    }
}

// ======================================================================
// SyntaxNodePtr
// ======================================================================

/// A stable, tree-independent reference to a node by its `kind` and absolute
/// `text_range`.
///
/// Because a green subtree can be reused across reparses, a raw
/// [`SyntaxNode`] from an old tree is worthless against a new one. A
/// `SyntaxNodePtr` survives the round trip: after an edit produces a new tree,
/// [`resolve`](Self::resolve) walks the new root to find "the same" node.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct SyntaxNodePtr {
    kind: SyntaxKind,
    range: TextRange,
}

impl SyntaxNodePtr {
    /// Captures `node`'s kind and range.
    pub fn new(node: &SyntaxNode) -> SyntaxNodePtr {
        SyntaxNodePtr {
            kind: node.kind(),
            range: node.text_range(),
        }
    }

    /// The captured node kind.
    #[inline]
    pub fn kind(&self) -> SyntaxKind {
        self.kind
    }

    /// The captured absolute range.
    #[inline]
    pub fn text_range(&self) -> TextRange {
        self.range
    }

    /// Descends from `root` for a node with the same kind and range.
    pub fn resolve(&self, root: &SyntaxNode) -> Option<SyntaxNode> {
        root.descendants()
            .find(|n| n.kind() == self.kind && n.text_range() == self.range)
    }
}

// ======================================================================
// Tests
// ======================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use crate::green::{GreenBuilder, GreenNode, GreenToken, GreenTrivia};
    use crate::kind::SyntaxKind as K;

    fn ts(n: u32) -> TextSize {
        TextSize::new(n)
    }

    fn tr(a: u32, b: u32) -> TextRange {
        TextRange::new(ts(a), ts(b))
    }

    fn ws(text: &str) -> GreenTrivia {
        GreenTrivia::new(K::WHITESPACE, text)
    }

    /// Builds the tree for `*start\n  hi\n`:
    ///
    /// ```text
    /// SCENARIO
    ///   LABEL_LINE   "*start\n"
    ///     STAR "*"
    ///     LABEL_NAME
    ///       TEXT "start"
    ///     NEWLINE "\n"
    ///   TEXT_LINE    "  hi\n"
    ///     TEXT  (leading "  ") "hi"
    ///     NEWLINE "\n"
    /// ```
    fn sample() -> SyntaxNode {
        let mut b = GreenBuilder::new();
        b.start_node(K::SCENARIO);

        b.start_node(K::LABEL_LINE);
        b.token(K::STAR, "*", vec![], vec![]);
        b.start_node(K::LABEL_NAME);
        b.token(K::TEXT, "start", vec![], vec![]);
        b.finish_node();
        b.token(K::NEWLINE, "\n", vec![], vec![]);
        b.finish_node();

        b.start_node(K::TEXT_LINE);
        let lead = b.trivia(K::WHITESPACE, "  ");
        b.token(K::TEXT, "hi", vec![lead], vec![]);
        b.token(K::NEWLINE, "\n", vec![], vec![]);
        b.finish_node();

        b.finish_node();
        SyntaxNode::new_root(b.finish())
    }

    #[test]
    fn root_ranges_and_text() {
        let root = sample();
        assert_eq!(root.kind(), K::SCENARIO);
        assert_eq!(root.text_range(), tr(0, 12));
        assert_eq!(root.text(), "*start\n  hi\n");
        assert!(root.parent().is_none());
    }

    #[test]
    fn text_range_vs_trimmed_range() {
        let root = sample();
        let text_line = root.children().nth(1).unwrap();
        assert_eq!(text_line.kind(), K::TEXT_LINE);
        // Full range includes the leading "  " trivia: "  hi\n" = 7..12.
        assert_eq!(text_line.text_range(), tr(7, 12));
        // Trimmed range excludes the first token's leading trivia: "hi\n".
        assert_eq!(text_line.trimmed_range(), tr(9, 12));
    }

    #[test]
    fn parent_and_ancestors() {
        let root = sample();
        let label_name = root
            .first_child()
            .unwrap()
            .children()
            .next()
            .unwrap();
        assert_eq!(label_name.kind(), K::LABEL_NAME);
        let kinds: Vec<_> = label_name.ancestors().map(|n| n.kind()).collect();
        assert_eq!(kinds, vec![K::LABEL_NAME, K::LABEL_LINE, K::SCENARIO]);
        assert_eq!(label_name.parent().unwrap().kind(), K::LABEL_LINE);
    }

    #[test]
    fn children_and_siblings() {
        let root = sample();
        let label = root.first_child().unwrap();
        assert_eq!(label.kind(), K::LABEL_LINE);
        assert_eq!(label.next_sibling().unwrap().kind(), K::TEXT_LINE);
        assert!(label.prev_sibling().is_none());
        let text_line = label.next_sibling().unwrap();
        assert_eq!(text_line.prev_sibling().unwrap().kind(), K::LABEL_LINE);

        // children_with_tokens over LABEL_LINE: STAR, LABEL_NAME, NEWLINE.
        let kinds: Vec<_> = label.children_with_tokens().map(|e| e.kind()).collect();
        assert_eq!(kinds, vec![K::STAR, K::LABEL_NAME, K::NEWLINE]);

        // sibling-or-token navigation across the LABEL_NAME node.
        let name = label.children().next().unwrap();
        assert_eq!(
            name.prev_sibling_or_token().unwrap().kind(),
            K::STAR
        );
        assert_eq!(
            name.next_sibling_or_token().unwrap().kind(),
            K::NEWLINE
        );
    }

    #[test]
    fn descendants_are_preorder() {
        let root = sample();
        let kinds: Vec<_> = root.descendants().map(|n| n.kind()).collect();
        assert_eq!(
            kinds,
            vec![
                K::SCENARIO,
                K::LABEL_LINE,
                K::LABEL_NAME,
                K::TEXT_LINE,
            ]
        );
    }

    #[test]
    fn descendants_with_tokens_preorder() {
        let root = sample();
        let kinds: Vec<_> = root.descendants_with_tokens().map(|e| e.kind()).collect();
        assert_eq!(
            kinds,
            vec![
                K::SCENARIO,
                K::LABEL_LINE,
                K::STAR,
                K::LABEL_NAME,
                K::TEXT,
                K::NEWLINE,
                K::TEXT_LINE,
                K::TEXT,
                K::NEWLINE,
            ]
        );
    }

    #[test]
    fn first_and_last_token() {
        let root = sample();
        assert_eq!(root.first_token().unwrap().kind(), K::STAR);
        assert_eq!(root.first_token().unwrap().text(), "*");
        let last = root.last_token().unwrap();
        assert_eq!(last.kind(), K::NEWLINE);
        assert_eq!(last.text_range(), tr(11, 12));
    }

    #[test]
    fn next_and_prev_token_cross_boundaries() {
        let root = sample();
        // Walk forward from the first token, collecting kinds.
        let mut cur = root.first_token();
        let mut kinds = Vec::new();
        while let Some(t) = cur {
            kinds.push(t.kind());
            cur = t.next_token();
        }
        assert_eq!(
            kinds,
            vec![K::STAR, K::TEXT, K::NEWLINE, K::TEXT, K::NEWLINE]
        );

        // prev_token from the last steps back across the node boundary.
        let last = root.last_token().unwrap();
        let prev = last.prev_token().unwrap();
        assert_eq!(prev.kind(), K::TEXT);
        assert_eq!(prev.text(), "hi");
        assert!(root.first_token().unwrap().prev_token().is_none());
        assert!(last.next_token().is_none());
    }

    #[test]
    fn token_ranges_and_trivia() {
        let root = sample();
        // The "hi" token with leading "  " trivia.
        let hi = root
            .children()
            .nth(1)
            .unwrap()
            .first_token()
            .unwrap();
        assert_eq!(hi.text(), "hi");
        assert_eq!(hi.full_range(), tr(7, 11)); // "  hi"
        assert_eq!(hi.text_range(), tr(9, 11)); // "hi"
        let lead = hi.leading_trivia_ranges();
        assert_eq!(lead, vec![(K::WHITESPACE, tr(7, 9))]);
        assert!(hi.trailing_trivia_ranges().is_empty());
    }

    #[test]
    fn trailing_trivia_ranges_are_absolute() {
        // A token "x" with leading " " and trailing "  ": offsets 0.." ",
        // "x" at 1, trailing at 2..4.
        let green = GreenToken::new(K::TEXT, "x", vec![ws(" ")], vec![ws("  ")]);
        let line = GreenNode::new(K::TEXT_LINE, vec![green.into()]);
        let root = SyntaxNode::new_root(GreenNode::new(K::SCENARIO, vec![line.into()]));
        let tok = root.first_token().unwrap();
        assert_eq!(tok.full_range(), tr(0, 4));
        assert_eq!(tok.text_range(), tr(1, 2));
        assert_eq!(
            tok.trailing_trivia_ranges(),
            vec![(K::WHITESPACE, tr(2, 4))]
        );
    }

    #[test]
    fn token_at_offset_single_and_between() {
        let root = sample();
        // Offsets: STAR[0,1) TEXT"start"[1,6) NEWLINE[6,7) then
        // TEXT"  hi" full[7,11) NEWLINE[11,12).
        // Strictly inside STAR text? STAR is [0,1); offset 0 is a boundary
        // (start of text) => Single(STAR).
        assert_eq!(
            root.token_at_offset(ts(0)),
            TokenAtOffset::Single(root.first_token().unwrap())
        );
        // Offset 3 is inside "start".
        match root.token_at_offset(ts(3)) {
            TokenAtOffset::Single(t) => assert_eq!(t.text(), "start"),
            other => panic!("expected Single, got {other:?}"),
        }
        // Offset 1 is the boundary between STAR and "start".
        match root.token_at_offset(ts(1)) {
            TokenAtOffset::Between(l, r) => {
                assert_eq!(l.text(), "*");
                assert_eq!(r.text(), "start");
            }
            other => panic!("expected Between, got {other:?}"),
        }
        // Offset 8 is inside the "hi" token's leading trivia "  " ([7,9)),
        // which belongs to that token's full range => Single("hi").
        match root.token_at_offset(ts(8)) {
            TokenAtOffset::Single(t) => assert_eq!(t.text(), "hi"),
            other => panic!("expected Single, got {other:?}"),
        }
        // Offset 7 is the boundary between the first NEWLINE and the "hi"
        // token (its leading trivia starts at 7).
        match root.token_at_offset(ts(7)) {
            TokenAtOffset::Between(l, r) => {
                assert_eq!(l.kind(), K::NEWLINE);
                assert_eq!(r.text(), "hi");
            }
            other => panic!("expected Between, got {other:?}"),
        }
        // End of text: Single(last).
        match root.token_at_offset(ts(12)) {
            TokenAtOffset::Single(t) => assert_eq!(t.text_range(), tr(11, 12)),
            other => panic!("expected Single, got {other:?}"),
        }
    }

    #[test]
    fn token_at_offset_biasing() {
        let root = sample();
        let at1 = root.token_at_offset(ts(1));
        assert_eq!(at1.clone().left_biased().unwrap().text(), "*");
        assert_eq!(at1.right_biased().unwrap().text(), "start");
    }

    #[test]
    fn covering_element_returns_parent_for_cross_sibling_range() {
        let root = sample();
        // A range spanning STAR ([0,1)) and into "start" resolves to the
        // LABEL_LINE parent.
        let el = root.covering_element(tr(0, 3));
        assert_eq!(el.kind(), K::LABEL_LINE);
        // A range fully inside "start" resolves to that token.
        let el = root.covering_element(tr(2, 4));
        assert_eq!(el.kind(), K::TEXT);
        assert!(el.as_token().is_some());
    }

    #[test]
    fn find_node_at_offset_smallest_node() {
        let root = sample();
        // Offset 3 is inside "start", whose enclosing node is LABEL_NAME.
        let node = root.find_node_at_offset(ts(3)).unwrap();
        assert_eq!(node.kind(), K::LABEL_NAME);
        // Out of range.
        assert!(root.find_node_at_offset(ts(99)).is_none());
    }

    #[test]
    fn missing_token_does_not_disturb_offsets() {
        // SCENARIO[ TEXT_LINE[ TEXT "hi", missing NEWLINE ] ]
        let mut b = GreenBuilder::new();
        b.start_node(K::SCENARIO);
        b.start_node(K::TEXT_LINE);
        b.token(K::TEXT, "hi", vec![], vec![]);
        b.missing_token(K::NEWLINE);
        b.finish_node();
        b.finish_node();
        let root = SyntaxNode::new_root(b.finish());
        assert_eq!(root.text(), "hi");
        assert_eq!(root.text_range(), tr(0, 2));
        // The missing token is zero-width and skipped by token_at_offset.
        match root.token_at_offset(ts(1)) {
            TokenAtOffset::Single(t) => assert_eq!(t.text(), "hi"),
            other => panic!("expected Single, got {other:?}"),
        }
    }

    #[test]
    fn syntax_node_ptr_resolves_after_rebuild() {
        let root = sample();
        let label_name = root
            .descendants()
            .find(|n| n.kind() == K::LABEL_NAME)
            .unwrap();
        let ptr = SyntaxNodePtr::new(&label_name);
        assert_eq!(ptr.kind(), K::LABEL_NAME);
        assert_eq!(ptr.text_range(), label_name.text_range());

        // Rebuild an identical tree from scratch and resolve against it.
        let rebuilt = sample();
        let resolved = ptr.resolve(&rebuilt).unwrap();
        assert_eq!(resolved.kind(), K::LABEL_NAME);
        assert_eq!(resolved.text_range(), label_name.text_range());
        assert_eq!(resolved.text(), "start");
    }

    #[test]
    fn equality_and_hash() {
        use std::collections::hash_map::DefaultHasher;

        let root = sample();
        let a = root.first_child().unwrap();
        let b = root.first_child().unwrap();
        assert_eq!(a, b);

        let hash = |n: &SyntaxNode| {
            let mut h = DefaultHasher::new();
            n.hash(&mut h);
            h.finish()
        };
        assert_eq!(hash(&a), hash(&b));
        // Different node, different identity.
        let other = root.children().nth(1).unwrap();
        assert_ne!(a, other);

        // Tokens compare and hash by content + position too.
        let t0 = root.first_token().unwrap();
        let t1 = root.first_token().unwrap();
        assert_eq!(t0, t1);
        let thash = |t: &SyntaxToken| {
            let mut h = DefaultHasher::new();
            t.hash(&mut h);
            h.finish()
        };
        assert_eq!(thash(&t0), thash(&t1));
    }

    #[test]
    fn element_accessors() {
        let root = sample();
        let el: SyntaxElement = root.first_child().unwrap().into();
        assert_eq!(el.kind(), K::LABEL_LINE);
        assert!(el.as_node().is_some());
        assert!(el.as_token().is_none());
        assert_eq!(el.index(), 0);
        assert_eq!(el.parent().unwrap().kind(), K::SCENARIO);
        assert_eq!(el.full_range(), el.text_range());

        let tok_el: SyntaxElement = root.first_token().unwrap().into();
        assert_eq!(tok_el.kind(), K::STAR);
        assert!(tok_el.into_token().is_some());
    }

    #[test]
    fn debug_and_display() {
        let root = sample();
        assert_eq!(format!("{root:?}"), "scenario@0..12");
        assert_eq!(format!("{root}"), "*start\n  hi\n");
        let star = root.first_token().unwrap();
        assert_eq!(format!("{star:?}"), "star@0..1 \"*\"");
        assert_eq!(format!("{star}"), "*");
    }

    #[test]
    fn send_sync() {
        fn assert_send_sync<T: Send + Sync>() {}
        assert_send_sync::<SyntaxNode>();
        assert_send_sync::<SyntaxToken>();
        assert_send_sync::<SyntaxElement>();
        assert_send_sync::<TokenAtOffset>();
        assert_send_sync::<SyntaxNodePtr>();
    }
}
