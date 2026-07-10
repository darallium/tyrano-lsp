//! Text-derived, memoized views of a [`SourceFile`](crate::SourceFile).
//!
//! Adapted from `ruff_db::source`. Ruff memoizes both the source text and
//! its line index; here the text *is* the salsa input, so only the line
//! index needs a query. Everything in this module depends on the file's
//! text alone: toggling parse or interpret options must never invalidate a
//! line index, because line/column mapping is a property of the bytes, not
//! of how they are parsed.

use std::sync::Arc;

use tyrano_syntax::text::{LineIndex, SourceText};

/// Line index for `file`, memoized.
///
/// Depends only on the text: changing parse or interpret options must not
/// invalidate it. (ruff_db has a `source_text` query too; here the text IS
/// the salsa input, so only the line index needs a query.)
#[salsa::tracked]
pub fn line_index(db: &dyn crate::Db, file: crate::SourceFile) -> Arc<LineIndex> {
    Arc::new(LineIndex::new(file.text(db)))
}

impl crate::SourceFile {
    /// A [`SourceText`] view of this file's current text (cheap `Arc`
    /// wrapper, lazy shared line index).
    ///
    /// Not a salsa query: [`SourceText`] is not `Eq`, and callers that only
    /// need line/column mapping should prefer [`line_index`], which IS
    /// memoized.
    pub fn source_text(self, db: &dyn crate::Db) -> SourceText {
        SourceText::new(self.text(db).clone())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{RootDatabase, SourceFile};
    use salsa::Setter as _;
    use tyrano_syntax::ParseOptions;
    use tyrano_syntax::text::{LineCol, TextSize};

    #[test]
    fn line_index_maps_offsets() {
        let db = RootDatabase::default();
        let file = SourceFile::with_defaults(&db, "ab\nc\n".to_string());
        let idx = line_index(&db, file);
        // Byte 3 is 'c', the first byte of line 1.
        assert_eq!(idx.line_col(TextSize::new(3)), LineCol { line: 1, col: 0 });
    }

    #[test]
    fn line_index_is_memoized() {
        let db = RootDatabase::default();
        let file = SourceFile::with_defaults(&db, "ab\nc\n".to_string());
        let a = line_index(&db, file);
        let b = line_index(&db, file);
        assert!(Arc::ptr_eq(&a, &b), "memo hit must return the same Arc");
    }

    #[test]
    fn text_change_recomputes() {
        let mut db = RootDatabase::default();
        let file = SourceFile::with_defaults(&db, "ab\nc\n".to_string());
        let before = line_index(&db, file);
        assert_eq!(before.line_count(), 3);
        file.set_text(&mut db).to("one\ntwo\nthree\nfour\n".to_string());
        let after = line_index(&db, file);
        assert_eq!(after.line_count(), 5);
    }

    #[test]
    fn parse_options_change_keeps_line_index() {
        let mut db = RootDatabase::default();
        let file = SourceFile::with_defaults(&db, "ab\nc\n".to_string());
        let before = line_index(&db, file);
        file.set_parse_options(&mut db)
            .to(ParseOptions { loose_endscript_termination: false });
        let after = line_index(&db, file);
        // Field-granular dependency: the line index does not read parse_options.
        assert!(Arc::ptr_eq(&before, &after), "options must not invalidate line_index");
    }

    #[test]
    fn source_text_roundtrip() {
        let db = RootDatabase::default();
        let text = "ab\nc\n";
        let file = SourceFile::with_defaults(&db, text.to_string());
        assert_eq!(file.source_text(&db).as_str(), text);
    }
}
