//! Variable **places** for TyranoScript expressions.
//!
//! TyranoScript stores game state in four engine-owned variable namespaces,
//! addressed in embedded expressions through the reserved roots `f.` (game
//! variables), `sf.` (system variables), `tf.` (temporary variables), and
//! `mp.` (macro parameters). A *place* is one access path rooted at such a
//! namespace, e.g. `f.hero.hp` reads game variable `hero.hp`.
//!
//! This module walks the expression trees produced by
//! [`tyrano_syntax::expr`] and extracts every place access, classifying each
//! as a [`AccessKind::Read`] or a [`AccessKind::Write`]. It is the bridge from
//! the syntactic expression layer to any later semantic analysis (unused-var
//! detection, rename, golden dumps of state usage).
//!
//! # What counts as a place
//!
//! A [`SyntaxKind::NAME_REF`] whose identifier is exactly one of `f`/`sf`/`tf`/
//! `mp`, standing alone or as the base of a `FIELD_EXPR`/`INDEX_EXPR` chain,
//! yields a [`Place`]. A bare name like `hero` is *not* a place. Field accesses
//! and literal index accesses become [`PathSeg::Field`] segments; a non-literal
//! index (`f.a[f.i]`) becomes a [`PathSeg::Dynamic`] segment — exact path
//! knowledge stops there, but the structural chain continues, and the inner
//! index expression (`f.i`) is collected as its own [`AccessKind::Read`] place.
//!
//! # Ranges
//!
//! Node ranges inside an [`ExprParse`] are **relative** to the expression text
//! (offset 0 is the first byte of the input; only the parse *diagnostics* are
//! absolutized). [`collect_places`] therefore adds its `anchor` argument to
//! every emitted range, so outputs are file-absolute.

use tyrano_syntax::SyntaxKind;
use tyrano_syntax::expr::ExprParse;
use tyrano_syntax::red::SyntaxNode;
use tyrano_syntax::text::{TextRange, TextSize};

/// One of the four engine variable namespaces a place can be rooted at.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum PlaceRoot {
    /// Game variables, addressed through `f.`.
    GameVar,
    /// System variables, addressed through `sf.`.
    SystemVar,
    /// Temporary variables, addressed through `tf.`.
    TempVar,
    /// Macro parameters, addressed through `mp.`.
    MacroParams,
}

impl PlaceRoot {
    /// Maps a root identifier to its namespace, or `None` if `ident` is not one
    /// of the reserved roots `f`/`sf`/`tf`/`mp`.
    pub fn from_ident(ident: &str) -> Option<PlaceRoot> {
        match ident {
            "f" => Some(PlaceRoot::GameVar),
            "sf" => Some(PlaceRoot::SystemVar),
            "tf" => Some(PlaceRoot::TempVar),
            "mp" => Some(PlaceRoot::MacroParams),
            _ => None,
        }
    }

    /// The reserved identifier for this namespace: `"f"`/`"sf"`/`"tf"`/`"mp"`.
    pub fn as_str(self) -> &'static str {
        match self {
            PlaceRoot::GameVar => "f",
            PlaceRoot::SystemVar => "sf",
            PlaceRoot::TempVar => "tf",
            PlaceRoot::MacroParams => "mp",
        }
    }
}

/// One segment of a place's access path.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum PathSeg {
    /// A statically known step: a `.name` field, or a literal index like
    /// `["key"]` / `[0]`. String literals are stored unquoted; numbers are
    /// stored as written.
    Field(String),
    /// A `[expr]` step with a non-literal key — exact path knowledge stops
    /// here, though the structural chain may continue past it.
    Dynamic,
}

/// A variable access path, e.g. `f.hero.hp` is [`PlaceRoot::GameVar`] with path
/// `[Field("hero"), Field("hp")]`.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Place {
    /// The namespace the path is rooted at.
    pub root: PlaceRoot,
    /// The access path from the root, in order.
    pub path: Vec<PathSeg>,
}

impl Place {
    /// Renders the place as a stable string for golden dumps.
    ///
    /// Every [`PathSeg::Field`] — whether it came from a `.name` field or a
    /// literal index — renders as `.name`, and every [`PathSeg::Dynamic`]
    /// renders as `[*]`. So `f.hero.hp` renders as `"f.hero.hp"`, `f.items[0]`
    /// as `"f.items.0"`, and `f.a[f.i].b` as `"f.a[*].b"`.
    pub fn render(&self) -> String {
        let mut out = String::from(self.root.as_str());
        for seg in &self.path {
            match seg {
                PathSeg::Field(name) => {
                    out.push('.');
                    out.push_str(name);
                }
                PathSeg::Dynamic => out.push_str("[*]"),
            }
        }
        out
    }
}

impl std::fmt::Display for Place {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.render())
    }
}

/// Whether a place access reads or writes the variable.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum AccessKind {
    /// The place is read.
    Read,
    /// The place is assigned to. A compound assignment (`+=`, `-=`, …) is a
    /// read-modify-write but is reported as a single `Write` in this version.
    Write,
}

/// Walks one parsed expression and appends every place access it finds.
///
/// `anchor` is the absolute byte offset of the expression text within the file;
/// it is added to the relative node ranges so every emitted [`TextRange`] is
/// file-absolute and covers the full path expression (from the root identifier
/// through the last path segment it covers).
///
/// Newly appended entries are sorted by range start (document order); any
/// entries already in `out` are left untouched.
pub fn collect_places(
    expr: &ExprParse,
    anchor: TextSize,
    out: &mut Vec<(Place, TextRange, AccessKind)>,
) {
    let start = out.len();
    collect_node(&expr.syntax(), anchor, out);
    out[start..].sort_by_key(|(_, range, _)| range.start());
}

/// The result of resolving a `FIELD_EXPR`/`INDEX_EXPR`/`NAME_REF` chain into a
/// place: the root namespace, the path, and the non-literal index-key
/// subexpressions that must still be searched for nested places.
struct Chain {
    root: PlaceRoot,
    path: Vec<PathSeg>,
    dynamic_keys: Vec<SyntaxNode>,
}

/// Recursively visits `node`, emitting the outermost place chains and then
/// recursing only where further places can hide (dynamic index keys and, for
/// non-chain nodes, all children).
fn collect_node(node: &SyntaxNode, anchor: TextSize, out: &mut Vec<(Place, TextRange, AccessKind)>) {
    if let Some(chain) = build_chain(node) {
        let range = node.trimmed_range() + anchor;
        let access = if is_write_lhs(node) { AccessKind::Write } else { AccessKind::Read };
        out.push((Place { root: chain.root, path: chain.path }, range, access));
        // The path is emitted as one place, but a dynamic key like the `f.i`
        // in `f.a[f.i]` is itself an access — recurse into those subtrees.
        for key in &chain.dynamic_keys {
            collect_node(key, anchor, out);
        }
        return;
    }

    for child in node.children() {
        collect_node(&child, anchor, out);
    }
}

/// Resolves `node` as a place chain, or `None` if it is not a `FIELD_EXPR`/
/// `INDEX_EXPR`/`NAME_REF` chain rooted at a reserved namespace.
fn build_chain(node: &SyntaxNode) -> Option<Chain> {
    match node.kind() {
        SyntaxKind::NAME_REF => {
            let ident = node.first_token()?;
            if ident.is_missing() {
                return None;
            }
            let root = PlaceRoot::from_ident(ident.text())?;
            Some(Chain { root, path: Vec::new(), dynamic_keys: Vec::new() })
        }
        SyntaxKind::FIELD_EXPR => {
            let base = node.first_child()?;
            let mut chain = build_chain(&base)?;
            let name = field_name(node);
            chain.path.push(PathSeg::Field(name));
            Some(chain)
        }
        SyntaxKind::INDEX_EXPR => {
            let base = node.first_child()?;
            let mut chain = build_chain(&base)?;
            // children() over INDEX_EXPR is [base, key]; the key is the second.
            let key = node.children().nth(1);
            match key {
                Some(k) if k.kind() == SyntaxKind::LITERAL => {
                    chain.path.push(PathSeg::Field(literal_text(&k)));
                }
                Some(k) => {
                    chain.path.push(PathSeg::Dynamic);
                    chain.dynamic_keys.push(k);
                }
                None => chain.path.push(PathSeg::Dynamic),
            }
            Some(chain)
        }
        _ => None,
    }
}

/// The field name of a `FIELD_EXPR`: its direct `IDENT` token, unquoted of
/// nothing (bare identifier). A missing name yields the empty string.
fn field_name(field_expr: &SyntaxNode) -> String {
    field_expr
        .children_with_tokens()
        .filter_map(|e| e.into_token())
        .find(|t| t.kind() == SyntaxKind::IDENT)
        .filter(|t| !t.is_missing())
        .map(|t| t.text().to_string())
        .unwrap_or_default()
}

/// The path text of a literal index key: a string literal with its surrounding
/// quotes stripped, or any other literal (number, keyword) as written.
fn literal_text(literal: &SyntaxNode) -> String {
    let Some(token) = literal.first_token() else {
        return String::new();
    };
    let text = token.text();
    if token.kind() == SyntaxKind::STRING && text.len() >= 2 {
        // Quotes (`"`, `'`, `` ` ``) are single ASCII bytes.
        text[1..text.len() - 1].to_string()
    } else {
        text.to_string()
    }
}

/// Whether `node` is the left-hand side of an assignment `BIN_EXPR` (`=`, `+=`,
/// `-=`, `*=`, `/=`, `%=`), which makes the place a write.
fn is_write_lhs(node: &SyntaxNode) -> bool {
    let Some(parent) = node.parent() else {
        return false;
    };
    if parent.kind() != SyntaxKind::BIN_EXPR {
        return false;
    }
    if !bin_operator(&parent).is_some_and(is_assignment_op) {
        return false;
    }
    // The LHS is the first (and only) node child before the operator token.
    parent.first_child().as_ref() == Some(node)
}

/// The operator token kind of a `BIN_EXPR` (its first direct token child).
fn bin_operator(bin_expr: &SyntaxNode) -> Option<SyntaxKind> {
    bin_expr
        .children_with_tokens()
        .find_map(|e| e.into_token())
        .map(|t| t.kind())
}

/// Whether `kind` is an assignment operator (simple `=` or a compound `+=` …).
/// Comparison operators (`==`, `!=`, …) and `,` are deliberately excluded.
fn is_assignment_op(kind: SyntaxKind) -> bool {
    matches!(
        kind,
        SyntaxKind::EQ
            | SyntaxKind::PLUS_EQ
            | SyntaxKind::MINUS_EQ
            | SyntaxKind::STAR_EQ
            | SyntaxKind::SLASH_EQ
            | SyntaxKind::PERCENT_EQ
    )
}

// ======================================================================
// Tests
// ======================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use tyrano_syntax::expr::parse_expr;

    /// Parses `src` at `anchor` and collects its places.
    fn places(src: &str, anchor: u32) -> Vec<(Place, TextRange, AccessKind)> {
        let parse = parse_expr(src, TextSize::new(anchor));
        let mut out = Vec::new();
        collect_places(&parse, TextSize::new(anchor), &mut out);
        out
    }

    fn field(name: &str) -> PathSeg {
        PathSeg::Field(name.to_string())
    }

    #[test]
    fn simple_field_chain() {
        let out = places("f.hero.hp", 0);
        assert_eq!(out.len(), 1);
        let (place, range, access) = &out[0];
        assert_eq!(place.root, PlaceRoot::GameVar);
        assert_eq!(place.path, vec![field("hero"), field("hp")]);
        assert_eq!(*access, AccessKind::Read);
        // Range covers the whole "f.hero.hp".
        assert_eq!(*range, TextRange::new(TextSize::new(0), TextSize::new(9)));
        assert_eq!(place.render(), "f.hero.hp");
    }

    #[test]
    fn all_roots_recognized() {
        for (src, root) in [
            ("f.a", PlaceRoot::GameVar),
            ("sf.a", PlaceRoot::SystemVar),
            ("tf.a", PlaceRoot::TempVar),
            ("mp.a", PlaceRoot::MacroParams),
        ] {
            let out = places(src, 0);
            assert_eq!(out.len(), 1, "for {src:?}");
            assert_eq!(out[0].0.root, root, "for {src:?}");
            assert_eq!(out[0].0.path, vec![field("a")], "for {src:?}");
        }
        // A bare root, standing alone, is still a place.
        let out = places("f", 0);
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].0.root, PlaceRoot::GameVar);
        assert!(out[0].0.path.is_empty());
    }

    #[test]
    fn plain_name_is_not_a_place() {
        assert!(places("hero.hp", 0).is_empty());
        assert!(places("hero", 0).is_empty());
        assert!(places("1 + 2", 0).is_empty());
    }

    #[test]
    fn literal_index_is_field() {
        let out = places("f.items[0]", 0);
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].0.path, vec![field("items"), field("0")]);
        assert_eq!(out[0].0.render(), "f.items.0");

        let out = places("f.map[\"key\"]", 0);
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].0.path, vec![field("map"), field("key")]);
    }

    #[test]
    fn dynamic_index_truncates_path() {
        let out = places("f.a[f.i].b", 0);
        assert_eq!(out.len(), 2);

        // Outer place: [a, Dynamic, b], Read, covering the whole expression.
        let (outer, outer_range, outer_access) = &out[0];
        assert_eq!(outer.path, vec![field("a"), PathSeg::Dynamic, field("b")]);
        assert_eq!(*outer_access, AccessKind::Read);
        assert_eq!(*outer_range, TextRange::new(TextSize::new(0), TextSize::new(10)));
        assert_eq!(outer.render(), "f.a[*].b");

        // Inner place from the dynamic key: f.i, Read.
        let (inner, inner_range, inner_access) = &out[1];
        assert_eq!(inner.root, PlaceRoot::GameVar);
        assert_eq!(inner.path, vec![field("i")]);
        assert_eq!(*inner_access, AccessKind::Read);
        assert_eq!(*inner_range, TextRange::new(TextSize::new(4), TextSize::new(7)));
    }

    #[test]
    fn assignment_lhs_is_write() {
        let out = places("f.x = 1", 0);
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].0.path, vec![field("x")]);
        assert_eq!(out[0].2, AccessKind::Write);

        // `+=` is supported by the expr parser (PLUS_EQ) and is a Write.
        let out = places("f.x += 1", 0);
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].0.path, vec![field("x")]);
        assert_eq!(out[0].2, AccessKind::Write);
    }

    #[test]
    fn both_sides_of_assignment() {
        let out = places("f.x = f.y", 0);
        assert_eq!(out.len(), 2);
        // Document order: f.x (start 0) then f.y (start 6).
        assert_eq!(out[0].0.path, vec![field("x")]);
        assert_eq!(out[0].2, AccessKind::Write);
        assert_eq!(out[1].0.path, vec![field("y")]);
        assert_eq!(out[1].2, AccessKind::Read);
    }

    #[test]
    fn ranges_are_anchor_absolute() {
        // Node ranges inside the tree are relative (start at 0)...
        let parse = parse_expr("f.hero.hp", TextSize::new(100));
        assert_eq!(parse.syntax().text_range().start(), TextSize::new(0));
        // ...so collect_places must add the anchor to make them absolute.
        let out = places("f.hero.hp", 100);
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].1, TextRange::new(TextSize::new(100), TextSize::new(109)));
    }

    #[test]
    fn places_in_calls_and_ternary() {
        let out = places("foo(f.a, sf.b ? f.c : 1)", 0);
        assert_eq!(out.len(), 3);
        // Three Reads in document order.
        assert_eq!(out[0].0.render(), "f.a");
        assert_eq!(out[1].0.render(), "sf.b");
        assert_eq!(out[2].0.render(), "f.c");
        assert!(out.iter().all(|(_, _, a)| *a == AccessKind::Read));
        // Ranges are strictly increasing by start.
        assert!(out[0].1.start() < out[1].1.start());
        assert!(out[1].1.start() < out[2].1.start());
    }
}
