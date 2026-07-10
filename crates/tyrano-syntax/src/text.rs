//! Byte-offset text primitives shared across the syntax stack.
//!
//! Every position in a TyranoScript source file is expressed as a **byte**
//! offset into the raw UTF-8 buffer. This module provides the small value
//! types that make such offsets ergonomic and hard to misuse:
//!
//! - [`TextSize`] — a `u32` newtype for an offset or a length.
//! - [`TextRange`] — a half-open `[start, end)` byte span.
//! - [`TextEdit`] and the [`apply_edits`]/[`normalize_edits`] helpers —
//!   describe and apply replacements against an *old* text buffer.
//! - [`LineIndex`] — a newline table for O(log n) offset ↔ line/column
//!   translation. Columns are UTF-8 **byte** columns, never grapheme or
//!   char columns.
//! - [`SourceText`] — an `Arc<str>` buffer with a lazily built line index,
//!   the canonical owner of a parsed document's text.
//!
//! Nothing here strips a byte-order mark or rewrites CRLF: offsets are raw
//! byte offsets into the buffer exactly as it was handed to the parser, so
//! that the tree stays byte-for-byte faithful to the input.

use std::iter::Sum;
use std::ops::{Add, AddAssign, Sub, SubAssign};
use std::sync::{Arc, OnceLock};

// ======================================================================
// TextSize
// ======================================================================

/// A byte offset into, or a byte length of, a text buffer.
///
/// Backed by a `u32`: TyranoScript scenarios never approach 4 GiB, and a
/// compact size keeps [`TextRange`] and green-tree nodes cheap to copy.
#[derive(Clone, Copy, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct TextSize(u32);

impl TextSize {
    /// Constructs a [`TextSize`] from a raw byte count.
    #[inline]
    pub const fn new(raw: u32) -> TextSize {
        TextSize(raw)
    }

    /// Returns the underlying `u32`.
    #[inline]
    pub const fn raw(self) -> u32 {
        self.0
    }

    /// Widens the offset to a `usize` for indexing into slices.
    #[inline]
    pub const fn to_usize(self) -> usize {
        self.0 as usize
    }

    /// The byte length of a string, as a [`TextSize`].
    ///
    /// # Panics
    /// Panics only if `text` is longer than `u32::MAX` bytes, which cannot
    /// happen for any real scenario file.
    #[inline]
    pub fn of(text: &str) -> TextSize {
        TextSize::try_from(text.len()).expect("text length exceeds u32::MAX")
    }

    /// Saturating-free checked subtraction: `None` on underflow.
    #[inline]
    pub const fn checked_sub(self, rhs: TextSize) -> Option<TextSize> {
        match self.0.checked_sub(rhs.0) {
            Some(v) => Some(TextSize(v)),
            None => None,
        }
    }

    /// Checked addition: `None` on `u32` overflow.
    #[inline]
    pub const fn checked_add(self, rhs: TextSize) -> Option<TextSize> {
        match self.0.checked_add(rhs.0) {
            Some(v) => Some(TextSize(v)),
            None => None,
        }
    }
}

impl From<u32> for TextSize {
    #[inline]
    fn from(raw: u32) -> TextSize {
        TextSize(raw)
    }
}

impl From<TextSize> for u32 {
    #[inline]
    fn from(size: TextSize) -> u32 {
        size.0
    }
}

impl From<TextSize> for usize {
    #[inline]
    fn from(size: TextSize) -> usize {
        size.0 as usize
    }
}

/// Fallible widening from `usize`. Fails (rather than panicking) when the
/// value does not fit in a `u32`.
impl TryFrom<usize> for TextSize {
    type Error = std::num::TryFromIntError;

    #[inline]
    fn try_from(value: usize) -> Result<TextSize, Self::Error> {
        Ok(TextSize(u32::try_from(value)?))
    }
}

impl Add for TextSize {
    type Output = TextSize;

    #[inline]
    fn add(self, rhs: TextSize) -> TextSize {
        TextSize(self.0 + rhs.0)
    }
}

impl Sub for TextSize {
    type Output = TextSize;

    /// # Panics
    /// Panics on underflow. Use [`TextSize::checked_sub`] to handle it.
    #[inline]
    fn sub(self, rhs: TextSize) -> TextSize {
        TextSize(
            self.0
                .checked_sub(rhs.0)
                .expect("TextSize subtraction underflowed"),
        )
    }
}

impl AddAssign for TextSize {
    #[inline]
    fn add_assign(&mut self, rhs: TextSize) {
        self.0 += rhs.0;
    }
}

impl SubAssign for TextSize {
    /// # Panics
    /// Panics on underflow.
    #[inline]
    fn sub_assign(&mut self, rhs: TextSize) {
        self.0 = self
            .0
            .checked_sub(rhs.0)
            .expect("TextSize subtraction underflowed");
    }
}

impl Sum for TextSize {
    #[inline]
    fn sum<I: Iterator<Item = TextSize>>(iter: I) -> TextSize {
        iter.fold(TextSize(0), Add::add)
    }
}

impl<'a> Sum<&'a TextSize> for TextSize {
    #[inline]
    fn sum<I: Iterator<Item = &'a TextSize>>(iter: I) -> TextSize {
        iter.copied().fold(TextSize(0), Add::add)
    }
}

impl std::fmt::Debug for TextSize {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl std::fmt::Display for TextSize {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

// ======================================================================
// TextRange
// ======================================================================

/// A half-open byte span `[start, end)` into a text buffer.
///
/// The invariant `start <= end` holds for every constructed value.
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub struct TextRange {
    start: TextSize,
    end: TextSize,
}

impl TextRange {
    /// Constructs a range from its endpoints.
    ///
    /// # Panics
    /// Panics if `start > end`.
    #[inline]
    pub fn new(start: TextSize, end: TextSize) -> TextRange {
        assert!(
            start <= end,
            "invalid TextRange: start {start} > end {end}"
        );
        TextRange { start, end }
    }

    /// Constructs a range from a start offset and a length.
    #[inline]
    pub fn at(offset: TextSize, len: TextSize) -> TextRange {
        TextRange::new(offset, offset + len)
    }

    /// An empty range positioned at `offset`.
    #[inline]
    pub fn empty(offset: TextSize) -> TextRange {
        TextRange {
            start: offset,
            end: offset,
        }
    }

    /// The inclusive start offset.
    #[inline]
    pub const fn start(self) -> TextSize {
        self.start
    }

    /// The exclusive end offset.
    #[inline]
    pub const fn end(self) -> TextSize {
        self.end
    }

    /// The length of the span in bytes.
    #[inline]
    pub fn len(self) -> TextSize {
        self.end - self.start
    }

    /// True when the span covers no bytes (`start == end`).
    #[inline]
    pub fn is_empty(self) -> bool {
        self.start == self.end
    }

    /// True when `offset` lies in `[start, end)` (half-open: `end` excluded).
    #[inline]
    pub fn contains(self, offset: TextSize) -> bool {
        self.start <= offset && offset < self.end
    }

    /// True when `offset` lies in `[start, end]` (both endpoints included).
    #[inline]
    pub fn contains_inclusive(self, offset: TextSize) -> bool {
        self.start <= offset && offset <= self.end
    }

    /// True when `other` is entirely inside `self` (endpoints may touch).
    #[inline]
    pub fn contains_range(self, other: TextRange) -> bool {
        self.start <= other.start && other.end <= self.end
    }

    /// The overlap of two ranges, or `None` if they do not overlap.
    ///
    /// Two ranges that merely touch at a point (e.g. `0..2` and `2..4`)
    /// intersect in the empty range at that point.
    #[inline]
    pub fn intersect(self, other: TextRange) -> Option<TextRange> {
        let start = self.start.max(other.start);
        let end = self.end.min(other.end);
        if start <= end {
            Some(TextRange { start, end })
        } else {
            None
        }
    }

    /// The smallest range containing both `self` and `other`.
    #[inline]
    pub fn cover(self, other: TextRange) -> TextRange {
        TextRange {
            start: self.start.min(other.start),
            end: self.end.max(other.end),
        }
    }

    /// Shifts the range right by `offset`, returning `None` on overflow.
    #[inline]
    pub fn checked_add(self, offset: TextSize) -> Option<TextRange> {
        Some(TextRange {
            start: self.start.checked_add(offset)?,
            end: self.end.checked_add(offset)?,
        })
    }

    /// Shifts the range left by `offset`, returning `None` on underflow.
    #[inline]
    pub fn checked_sub(self, offset: TextSize) -> Option<TextRange> {
        Some(TextRange {
            start: self.start.checked_sub(offset)?,
            end: self.end.checked_sub(offset)?,
        })
    }

    /// Slices `text` to this range.
    ///
    /// # Panics
    /// Panics if the range is out of bounds or does not fall on UTF-8 char
    /// boundaries — the same contract as `&text[start..end]`.
    #[inline]
    pub fn slice<'a>(&self, text: &'a str) -> &'a str {
        &text[self.start.to_usize()..self.end.to_usize()]
    }
}

impl Add<TextSize> for TextRange {
    type Output = TextRange;

    /// # Panics
    /// Panics on `u32` overflow. Use [`TextRange::checked_add`] to handle it.
    #[inline]
    fn add(self, offset: TextSize) -> TextRange {
        self.checked_add(offset)
            .expect("TextRange shift overflowed")
    }
}

impl Sub<TextSize> for TextRange {
    type Output = TextRange;

    /// # Panics
    /// Panics on underflow. Use [`TextRange::checked_sub`] to handle it.
    #[inline]
    fn sub(self, offset: TextSize) -> TextRange {
        self.checked_sub(offset)
            .expect("TextRange shift underflowed")
    }
}

impl std::fmt::Debug for TextRange {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{:?}..{:?}", self.start.raw(), self.end.raw())
    }
}

/// Slices `text` to `range`. Convenience free function mirroring
/// [`TextRange::slice`].
#[inline]
pub fn range_text(text: &str, range: TextRange) -> &str {
    range.slice(text)
}

// ======================================================================
// TextEdit
// ======================================================================

/// A single replacement expressed against the **old** text buffer.
///
/// `range` selects the byte span to remove and `replacement` is spliced in
/// its place. All edits in a batch address the *original* text, never the
/// intermediate result of applying earlier edits.
#[derive(Clone, PartialEq, Eq)]
pub struct TextEdit {
    /// The span in the old text that this edit replaces.
    pub range: TextRange,
    /// The text to splice in place of `range`.
    pub replacement: String,
}

impl TextEdit {
    /// An insertion at `offset` (an empty range with non-empty replacement).
    #[inline]
    pub fn insert(offset: TextSize, text: impl Into<String>) -> TextEdit {
        TextEdit {
            range: TextRange::empty(offset),
            replacement: text.into(),
        }
    }

    /// A deletion of `range` (an empty replacement).
    #[inline]
    pub fn delete(range: TextRange) -> TextEdit {
        TextEdit {
            range,
            replacement: String::new(),
        }
    }

    /// A replacement of `range` with `text`.
    #[inline]
    pub fn replace(range: TextRange, text: impl Into<String>) -> TextEdit {
        TextEdit {
            range,
            replacement: text.into(),
        }
    }
}

impl std::fmt::Debug for TextEdit {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{:?} => {:?}", self.range, self.replacement)
    }
}

/// Applies a batch of edits to `old`, producing the new text.
///
/// The edits are sorted by start offset and applied against the *original*
/// buffer. The edits must be non-overlapping, in bounds, and land on UTF-8
/// char boundaries.
///
/// # Panics
/// Panics with a descriptive message if any edit is out of bounds, straddles
/// a char boundary, or overlaps another edit.
pub fn apply_edits(old: &str, edits: &[TextEdit]) -> String {
    if edits.is_empty() {
        return old.to_owned();
    }

    let mut sorted: Vec<&TextEdit> = edits.iter().collect();
    sorted.sort_by_key(|e| (e.range.start(), e.range.end()));

    let mut result = String::with_capacity(old.len());
    let mut cursor = TextSize::new(0);

    for edit in sorted {
        let start = edit.range.start();
        let end = edit.range.end();

        assert!(
            end.to_usize() <= old.len(),
            "edit {edit:?} is out of bounds (text is {} bytes)",
            old.len()
        );
        assert!(
            start >= cursor,
            "edit {edit:?} overlaps a previous edit at offset {cursor}"
        );
        assert!(
            old.is_char_boundary(start.to_usize()),
            "edit {edit:?} starts inside a UTF-8 code point"
        );
        assert!(
            old.is_char_boundary(end.to_usize()),
            "edit {edit:?} ends inside a UTF-8 code point"
        );

        result.push_str(&old[cursor.to_usize()..start.to_usize()]);
        result.push_str(&edit.replacement);
        cursor = end;
    }

    result.push_str(&old[cursor.to_usize()..]);
    result
}

/// Sorts and merges a batch of edits, coalescing runs that touch or abut.
///
/// Edits whose spans touch (`a.end == b.start`) are merged into a single
/// edit covering the union of their ranges, concatenating their
/// replacements. The returned edits are sorted and pairwise disjoint.
///
/// # Panics
/// Panics if two edits truly overlap (`b.start < a.end`).
pub fn normalize_edits(edits: Vec<TextEdit>) -> Vec<TextEdit> {
    if edits.is_empty() {
        return edits;
    }

    let mut sorted = edits;
    sorted.sort_by_key(|e| (e.range.start(), e.range.end()));

    let mut merged: Vec<TextEdit> = Vec::with_capacity(sorted.len());
    for edit in sorted {
        match merged.last_mut() {
            Some(last) if edit.range.start() < last.range.end() => {
                panic!(
                    "overlapping edits: {:?} and {:?}",
                    last.range, edit.range
                );
            }
            Some(last) if edit.range.start() == last.range.end() => {
                // Touching edits: extend the previous span and concatenate.
                last.range = last.range.cover(edit.range);
                last.replacement.push_str(&edit.replacement);
            }
            _ => merged.push(edit),
        }
    }
    merged
}

// ======================================================================
// LineIndex
// ======================================================================

/// A 0-based line/column position.
///
/// `col` is a UTF-8 **byte** column: the number of bytes between the start
/// of the line and the position, *not* a count of characters or graphemes.
/// This keeps offset ↔ position translation exact and allocation-free; a
/// display layer can convert to a char/grapheme column if it needs one.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Debug)]
pub struct LineCol {
    /// 0-based line number.
    pub line: u32,
    /// 0-based UTF-8 byte offset within the line.
    pub col: u32,
}

/// A table of line-start byte offsets for fast offset ↔ line/column lookup.
///
/// The table always contains at least one entry (offset 0 for the first
/// line) and one additional entry immediately after every `\n`. A `\r` in a
/// CRLF sequence is treated as an ordinary byte belonging to the preceding
/// line; only `\n` begins a new line.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct LineIndex {
    /// Byte offset at which each line begins. `line_starts[0] == 0`.
    line_starts: Vec<TextSize>,
    /// Total byte length of the indexed text.
    len: TextSize,
}

impl LineIndex {
    /// Builds a line index for `text`.
    pub fn new(text: &str) -> LineIndex {
        let mut line_starts = Vec::with_capacity(text.len() / 32 + 1);
        line_starts.push(TextSize::new(0));
        for (i, b) in text.bytes().enumerate() {
            if b == b'\n' {
                // The next line starts on the byte after the newline.
                line_starts.push(TextSize::new(i as u32 + 1));
            }
        }
        LineIndex {
            line_starts,
            len: TextSize::of(text),
        }
    }

    /// Translates a byte offset to a line/column position.
    ///
    /// Offsets past the end of the text clamp to the final line, with the
    /// column measured from that line's start.
    pub fn line_col(&self, offset: TextSize) -> LineCol {
        let offset = offset.min(self.len);
        // Find the last line whose start is <= offset.
        let line = match self.line_starts.binary_search(&offset) {
            Ok(exact) => exact,
            Err(next) => next - 1,
        };
        let line_start = self.line_starts[line];
        LineCol {
            line: line as u32,
            col: (offset - line_start).raw(),
        }
    }

    /// Translates a line/column position back to a byte offset.
    ///
    /// Returns `None` if the line does not exist or the column runs past the
    /// end of that line (its newline included, if any).
    pub fn offset(&self, pos: LineCol) -> Option<TextSize> {
        let line_range = self.line_range(pos.line)?;
        let offset = line_range.start() + TextSize::new(pos.col);
        if offset <= line_range.end() {
            Some(offset)
        } else {
            None
        }
    }

    /// The number of lines in the text (always at least 1).
    #[inline]
    pub fn line_count(&self) -> u32 {
        self.line_starts.len() as u32
    }

    /// The byte range of `line`, **including** its terminating newline if it
    /// has one. The final line runs to the end of the text.
    ///
    /// Returns `None` if `line` is out of range.
    pub fn line_range(&self, line: u32) -> Option<TextRange> {
        let line = line as usize;
        let start = *self.line_starts.get(line)?;
        let end = self
            .line_starts
            .get(line + 1)
            .copied()
            .unwrap_or(self.len);
        Some(TextRange::new(start, end))
    }
}

// ======================================================================
// SourceText
// ======================================================================

/// An immutable, shareable source buffer with a lazily built line index.
///
/// Cloning is cheap: the underlying `Arc<str>` and the shared line-index
/// cell are reference-counted, so all clones observe the same text and
/// share the once-built [`LineIndex`].
#[derive(Clone)]
pub struct SourceText {
    text: Arc<str>,
    line_index: Arc<OnceLock<LineIndex>>,
}

impl SourceText {
    /// Wraps `text` as a source buffer. The byte-order mark, if present, is
    /// preserved; offsets are raw byte offsets into the given bytes.
    pub fn new(text: impl Into<Arc<str>>) -> SourceText {
        SourceText {
            text: text.into(),
            line_index: Arc::new(OnceLock::new()),
        }
    }

    /// The buffer as a string slice.
    #[inline]
    pub fn as_str(&self) -> &str {
        &self.text
    }

    /// The total byte length of the buffer.
    #[inline]
    pub fn len(&self) -> TextSize {
        TextSize::of(&self.text)
    }

    /// True when the buffer is empty.
    #[inline]
    pub fn is_empty(&self) -> bool {
        self.text.is_empty()
    }

    /// The full byte range `0..len` of the buffer.
    #[inline]
    pub fn text_range(&self) -> TextRange {
        TextRange::new(TextSize::new(0), self.len())
    }

    /// The line index, building it on first use and caching it thereafter.
    #[inline]
    pub fn line_index(&self) -> &LineIndex {
        self.line_index
            .get_or_init(|| LineIndex::new(&self.text))
    }

    /// Convenience: the line/column of `offset` via the cached line index.
    #[inline]
    pub fn line_col(&self, offset: TextSize) -> LineCol {
        self.line_index().line_col(offset)
    }
}

impl std::fmt::Debug for SourceText {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SourceText")
            .field("len", &self.len().raw())
            .finish_non_exhaustive()
    }
}

// ======================================================================
// Tests
// ======================================================================

#[cfg(test)]
mod tests {
    use super::*;

    fn ts(n: u32) -> TextSize {
        TextSize::new(n)
    }

    fn tr(start: u32, end: u32) -> TextRange {
        TextRange::new(ts(start), ts(end))
    }

    // -- TextSize ------------------------------------------------------

    #[test]
    fn size_basics() {
        let s = TextSize::new(5);
        assert_eq!(s.raw(), 5);
        assert_eq!(s.to_usize(), 5);
        assert_eq!(u32::from(s), 5);
        assert_eq!(usize::from(s), 5);
        assert_eq!(TextSize::from(7u32).raw(), 7);
        assert_eq!(TextSize::default(), ts(0));
    }

    #[test]
    fn size_of_str() {
        assert_eq!(TextSize::of("").raw(), 0);
        assert_eq!(TextSize::of("abc").raw(), 3);
        // Each Japanese char is 3 bytes in UTF-8.
        assert_eq!(TextSize::of("あ").raw(), 3);
    }

    #[test]
    fn size_try_from_usize() {
        assert_eq!(TextSize::try_from(10usize).unwrap(), ts(10));
        assert!(TextSize::try_from(usize::MAX).is_err());
    }

    #[test]
    fn size_arithmetic() {
        assert_eq!(ts(3) + ts(4), ts(7));
        assert_eq!(ts(7) - ts(4), ts(3));
        let mut s = ts(2);
        s += ts(3);
        assert_eq!(s, ts(5));
        s -= ts(1);
        assert_eq!(s, ts(4));
        assert_eq!(ts(2).checked_sub(ts(5)), None);
        assert_eq!(ts(5).checked_sub(ts(2)), Some(ts(3)));
        assert_eq!(ts(u32::MAX).checked_add(ts(1)), None);
    }

    #[test]
    #[should_panic(expected = "underflow")]
    fn size_sub_underflow_panics() {
        let _ = ts(1) - ts(2);
    }

    #[test]
    fn size_sum() {
        let total: TextSize = [ts(1), ts(2), ts(3)].into_iter().sum();
        assert_eq!(total, ts(6));
        let total_ref: TextSize = [ts(4), ts(5)].iter().sum();
        assert_eq!(total_ref, ts(9));
    }

    #[test]
    fn size_display_and_debug() {
        assert_eq!(format!("{}", ts(42)), "42");
        assert_eq!(format!("{:?}", ts(42)), "42");
    }

    // -- TextRange -----------------------------------------------------

    #[test]
    fn range_basics() {
        let r = tr(2, 5);
        assert_eq!(r.start(), ts(2));
        assert_eq!(r.end(), ts(5));
        assert_eq!(r.len(), ts(3));
        assert!(!r.is_empty());
        assert_eq!(TextRange::at(ts(2), ts(3)), r);
        assert_eq!(TextRange::empty(ts(4)), tr(4, 4));
        assert!(tr(4, 4).is_empty());
    }

    #[test]
    #[should_panic(expected = "invalid TextRange")]
    fn range_new_rejects_inverted() {
        let _ = tr(5, 2);
    }

    #[test]
    fn range_debug() {
        assert_eq!(format!("{:?}", tr(2, 5)), "2..5");
    }

    #[test]
    fn range_contains_half_open() {
        let r = tr(2, 5);
        assert!(!r.contains(ts(1)));
        assert!(r.contains(ts(2)));
        assert!(r.contains(ts(4)));
        assert!(!r.contains(ts(5))); // end excluded
        assert!(r.contains_inclusive(ts(5)));
        assert!(!r.contains_inclusive(ts(6)));
        // Empty range contains no offset via half-open contains.
        assert!(!tr(3, 3).contains(ts(3)));
        assert!(tr(3, 3).contains_inclusive(ts(3)));
    }

    #[test]
    fn range_contains_range() {
        let r = tr(2, 8);
        assert!(r.contains_range(tr(3, 6)));
        assert!(r.contains_range(tr(2, 8)));
        assert!(r.contains_range(tr(2, 2)));
        assert!(!r.contains_range(tr(1, 4)));
        assert!(!r.contains_range(tr(6, 9)));
    }

    #[test]
    fn range_intersect() {
        assert_eq!(tr(2, 6).intersect(tr(4, 8)), Some(tr(4, 6)));
        assert_eq!(tr(2, 6).intersect(tr(6, 8)), Some(tr(6, 6))); // touching
        assert_eq!(tr(2, 6).intersect(tr(7, 8)), None); // disjoint
        assert_eq!(tr(2, 10).intersect(tr(4, 6)), Some(tr(4, 6)));
    }

    #[test]
    fn range_cover() {
        assert_eq!(tr(2, 6).cover(tr(8, 10)), tr(2, 10));
        assert_eq!(tr(8, 10).cover(tr(2, 6)), tr(2, 10));
        assert_eq!(tr(2, 6).cover(tr(3, 4)), tr(2, 6));
    }

    #[test]
    fn range_shifting() {
        assert_eq!(tr(2, 6) + ts(3), tr(5, 9));
        assert_eq!(tr(5, 9) - ts(3), tr(2, 6));
        assert_eq!(tr(0, 2).checked_sub(ts(1)), None);
        assert_eq!(tr(2, 6).checked_add(ts(1)), Some(tr(3, 7)));
    }

    #[test]
    fn range_slice() {
        let text = "hello world";
        assert_eq!(tr(0, 5).slice(text), "hello");
        assert_eq!(tr(6, 11).slice(text), "world");
        assert_eq!(range_text(text, tr(6, 11)), "world");
    }

    // -- TextEdit ------------------------------------------------------

    #[test]
    fn edit_constructors() {
        assert_eq!(
            TextEdit::insert(ts(3), "x"),
            TextEdit {
                range: tr(3, 3),
                replacement: "x".into()
            }
        );
        assert_eq!(
            TextEdit::delete(tr(1, 4)),
            TextEdit {
                range: tr(1, 4),
                replacement: String::new()
            }
        );
        assert_eq!(
            TextEdit::replace(tr(1, 4), "yy"),
            TextEdit {
                range: tr(1, 4),
                replacement: "yy".into()
            }
        );
    }

    #[test]
    fn apply_edits_empty() {
        assert_eq!(apply_edits("abc", &[]), "abc");
    }

    #[test]
    fn apply_edits_multiple_disjoint() {
        // old: "hello world"
        //       0123456789A
        // insert "! " at 0, delete "llo" (2..5), replace "world" (6..11) with "there"
        let old = "hello world";
        let edits = vec![
            TextEdit::insert(ts(0), "! "),
            TextEdit::delete(tr(2, 5)),
            TextEdit::replace(tr(6, 11), "there"),
        ];
        // Manually: "! " + "he" + "" + " " + "there" = "! he there"
        assert_eq!(apply_edits(old, &edits), "! he there");
    }

    #[test]
    fn apply_edits_unsorted_input() {
        let old = "abcdef";
        let edits = vec![
            TextEdit::replace(tr(4, 6), "EF"),
            TextEdit::replace(tr(0, 2), "AB"),
        ];
        assert_eq!(apply_edits(old, &edits), "ABcdEF");
    }

    #[test]
    #[should_panic(expected = "overlaps")]
    fn apply_edits_overlap_panics() {
        let edits = vec![
            TextEdit::replace(tr(0, 4), "x"),
            TextEdit::replace(tr(2, 6), "y"),
        ];
        let _ = apply_edits("abcdefgh", &edits);
    }

    #[test]
    #[should_panic(expected = "out of bounds")]
    fn apply_edits_out_of_bounds_panics() {
        let _ = apply_edits("abc", &[TextEdit::delete(tr(2, 9))]);
    }

    #[test]
    #[should_panic(expected = "UTF-8")]
    fn apply_edits_char_boundary_panics() {
        // "あ" is 3 bytes; slicing at byte 1 straddles the code point.
        let _ = apply_edits("あ", &[TextEdit::delete(tr(0, 1))]);
    }

    #[test]
    fn normalize_edits_merges_touching() {
        let edits = vec![
            TextEdit::replace(tr(2, 4), "B"),
            TextEdit::replace(tr(0, 2), "A"),
        ];
        let merged = normalize_edits(edits);
        assert_eq!(
            merged,
            vec![TextEdit {
                range: tr(0, 4),
                replacement: "AB".into()
            }]
        );
    }

    #[test]
    fn normalize_edits_keeps_disjoint() {
        let edits = vec![
            TextEdit::replace(tr(0, 2), "A"),
            TextEdit::replace(tr(4, 6), "C"),
        ];
        let merged = normalize_edits(edits.clone());
        assert_eq!(merged, edits);
    }

    #[test]
    fn normalize_edits_empty() {
        assert_eq!(normalize_edits(vec![]), vec![]);
    }

    #[test]
    #[should_panic(expected = "overlapping")]
    fn normalize_edits_overlap_panics() {
        let edits = vec![
            TextEdit::replace(tr(0, 4), "A"),
            TextEdit::replace(tr(2, 6), "B"),
        ];
        let _ = normalize_edits(edits);
    }

    // -- LineIndex -----------------------------------------------------

    #[test]
    fn line_index_empty() {
        let idx = LineIndex::new("");
        assert_eq!(idx.line_count(), 1);
        assert_eq!(idx.line_col(ts(0)), LineCol { line: 0, col: 0 });
        assert_eq!(idx.line_range(0), Some(tr(0, 0)));
        assert_eq!(idx.line_range(1), None);
    }

    #[test]
    fn line_index_single_char() {
        let idx = LineIndex::new("a");
        assert_eq!(idx.line_count(), 1);
        assert_eq!(idx.line_col(ts(0)), LineCol { line: 0, col: 0 });
        assert_eq!(idx.line_col(ts(1)), LineCol { line: 0, col: 1 });
        assert_eq!(idx.line_range(0), Some(tr(0, 1)));
    }

    #[test]
    fn line_index_trailing_newline() {
        // "a\n" -> line 0 = "a\n" (0..2), line 1 = "" (2..2)
        let idx = LineIndex::new("a\n");
        assert_eq!(idx.line_count(), 2);
        assert_eq!(idx.line_range(0), Some(tr(0, 2)));
        assert_eq!(idx.line_range(1), Some(tr(2, 2)));
        assert_eq!(idx.line_col(ts(0)), LineCol { line: 0, col: 0 });
        assert_eq!(idx.line_col(ts(1)), LineCol { line: 0, col: 1 });
        assert_eq!(idx.line_col(ts(2)), LineCol { line: 1, col: 0 });
    }

    #[test]
    fn line_index_crlf() {
        // "a\r\nb": bytes a(0) \r(1) \n(2) b(3). \n at index 2 -> line 1 starts at 3.
        let idx = LineIndex::new("a\r\nb");
        assert_eq!(idx.line_count(), 2);
        // The \r belongs to line 0.
        assert_eq!(idx.line_range(0), Some(tr(0, 3))); // "a\r\n"
        assert_eq!(idx.line_range(1), Some(tr(3, 4))); // "b"
        assert_eq!(idx.line_col(ts(1)), LineCol { line: 0, col: 1 }); // \r
        assert_eq!(idx.line_col(ts(3)), LineCol { line: 1, col: 0 }); // b
    }

    #[test]
    fn line_index_multibyte_byte_columns() {
        // "あい\nう": each kana is 3 bytes. Line 0 = "あい\n" (0..7).
        let text = "あい\nう";
        let idx = LineIndex::new(text);
        assert_eq!(idx.line_count(), 2);
        // Column is a BYTE column: second kana starts at byte 3.
        assert_eq!(idx.line_col(ts(3)), LineCol { line: 0, col: 3 });
        // Newline is at byte 6.
        assert_eq!(idx.line_col(ts(6)), LineCol { line: 0, col: 6 });
        // "う" starts line 1 at byte 7.
        assert_eq!(idx.line_col(ts(7)), LineCol { line: 1, col: 0 });
        assert_eq!(idx.line_range(0), Some(tr(0, 7)));
        assert_eq!(idx.line_range(1), Some(tr(7, 10)));
    }

    #[test]
    fn line_index_clamps_past_end() {
        let idx = LineIndex::new("ab");
        // Offset past end clamps to the last line.
        assert_eq!(idx.line_col(ts(99)), LineCol { line: 0, col: 2 });
    }

    #[test]
    fn line_index_offset_roundtrip() {
        let text = "hello\nあ\nworld\n";
        let idx = LineIndex::new(text);
        for offset in 0..=text.len() as u32 {
            // Only roundtrip on char boundaries to keep col meaningful.
            if !text.is_char_boundary(offset as usize) {
                continue;
            }
            let lc = idx.line_col(ts(offset));
            assert_eq!(
                idx.offset(lc),
                Some(ts(offset)),
                "roundtrip failed at offset {offset} ({lc:?})"
            );
        }
    }

    #[test]
    fn line_index_offset_out_of_range() {
        let idx = LineIndex::new("ab\ncd");
        assert_eq!(idx.offset(LineCol { line: 9, col: 0 }), None);
        // col past end of line (line 0 is "ab\n" = 3 bytes, so col 4 overshoots).
        assert_eq!(idx.offset(LineCol { line: 0, col: 4 }), None);
        // col exactly at the newline boundary is allowed.
        assert_eq!(idx.offset(LineCol { line: 0, col: 3 }), Some(ts(3)));
    }

    // -- SourceText ----------------------------------------------------

    #[test]
    fn source_text_basics() {
        let src = SourceText::new("hello\nworld");
        assert_eq!(src.as_str(), "hello\nworld");
        assert_eq!(src.len(), ts(11));
        assert!(!src.is_empty());
        assert_eq!(src.text_range(), tr(0, 11));
        assert_eq!(src.line_col(ts(6)), LineCol { line: 1, col: 0 });
    }

    #[test]
    fn source_text_empty() {
        let src = SourceText::new("");
        assert!(src.is_empty());
        assert_eq!(src.len(), ts(0));
        assert_eq!(src.text_range(), tr(0, 0));
    }

    #[test]
    fn source_text_line_index_is_cached() {
        let src = SourceText::new("a\nb\nc");
        let first = src.line_index() as *const LineIndex;
        let second = src.line_index() as *const LineIndex;
        assert_eq!(first, second, "line index should be built once");
    }

    #[test]
    fn source_text_clone_shares_index() {
        let src = SourceText::new("a\nb");
        // Force the index to build on the original.
        let orig_ptr = src.line_index() as *const LineIndex;
        let clone = src.clone();
        // Clone shares the same OnceLock, so it observes the built index.
        let clone_ptr = clone.line_index() as *const LineIndex;
        assert_eq!(orig_ptr, clone_ptr);
        assert_eq!(clone.as_str(), "a\nb");
    }

    #[test]
    fn source_text_preserves_bom() {
        let with_bom = "\u{feff}hello";
        let src = SourceText::new(with_bom);
        // BOM is 3 bytes and is NOT stripped.
        assert_eq!(src.as_str(), with_bom);
        assert_eq!(src.len(), ts(8));
        assert_eq!(src.line_col(ts(3)), LineCol { line: 0, col: 3 });
    }
}
