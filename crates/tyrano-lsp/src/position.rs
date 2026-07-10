//! Byte offset ↔ LSP position translation.
//!
//! The pipeline's [`LineCol`] columns are UTF-8 *byte* columns; LSP
//! positions (with the default `utf-16` encoding this server advertises)
//! count UTF-16 code units. These helpers convert exactly, clamping
//! out-of-range client positions the way the LSP spec asks (past-the-end
//! columns clamp to the line end).
//!
//! [`LineCol`]: tyrano_syntax::text::LineCol

use lsp_types::Position;
use tyrano_syntax::text::{LineIndex, TextRange, TextSize};

/// Translates a byte offset into an LSP (UTF-16) position.
///
/// Offsets past the end of the text clamp to the final position.
pub fn offset_to_position(text: &str, index: &LineIndex, offset: TextSize) -> Position {
    let offset = offset.min(TextSize::of(text));
    let lc = index.line_col(offset);
    let line_start = usize::from(offset) - lc.col as usize;
    let prefix = &text[line_start..usize::from(offset)];
    Position::new(lc.line, prefix.encode_utf16().count() as u32)
}

/// Translates an LSP (UTF-16) position into a byte offset.
///
/// Columns past the end of the line clamp to the line end (excluding its
/// newline); lines past the end of the file yield `None`.
pub fn position_to_offset(text: &str, index: &LineIndex, pos: Position) -> Option<TextSize> {
    let line_range = index.line_range(pos.line)?;
    let line_text = &text[usize::from(line_range.start())..usize::from(line_range.end())];
    let content = line_text.trim_end_matches(['\n', '\r']);

    let mut utf16_units = 0u32;
    for (byte_col, ch) in content.char_indices() {
        if utf16_units >= pos.character {
            return Some(line_range.start() + TextSize::new(byte_col as u32));
        }
        utf16_units += ch.len_utf16() as u32;
    }
    Some(line_range.start() + TextSize::of(content))
}

/// Translates a byte range into an LSP range.
pub fn range_to_lsp(text: &str, index: &LineIndex, range: TextRange) -> lsp_types::Range {
    lsp_types::Range::new(
        offset_to_position(text, index, range.start()),
        offset_to_position(text, index, range.end()),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn index(text: &str) -> LineIndex {
        LineIndex::new(text)
    }

    #[test]
    fn ascii_round_trip() {
        let text = "abc\ndef\n";
        let idx = index(text);
        let pos = offset_to_position(text, &idx, TextSize::new(5));
        assert_eq!(pos, Position::new(1, 1));
        assert_eq!(position_to_offset(text, &idx, pos), Some(TextSize::new(5)));
    }

    #[test]
    fn multibyte_columns_count_utf16_units() {
        // "こんにちは" is 5 chars × 3 bytes, 1 UTF-16 unit each.
        let text = "こんにちは[l]\n";
        let idx = index(text);
        let offset = TextSize::new(15); // byte offset of '['
        let pos = offset_to_position(text, &idx, offset);
        assert_eq!(pos, Position::new(0, 5));
        assert_eq!(position_to_offset(text, &idx, pos), Some(offset));
    }

    #[test]
    fn surrogate_pairs_count_two_units() {
        // '𠮷' is 4 bytes in UTF-8 and 2 UTF-16 units.
        let text = "𠮷x\n";
        let idx = index(text);
        let x = TextSize::new(4);
        let pos = offset_to_position(text, &idx, x);
        assert_eq!(pos, Position::new(0, 2));
        assert_eq!(position_to_offset(text, &idx, pos), Some(x));
    }

    #[test]
    fn column_past_line_end_clamps() {
        let text = "ab\ncd\n";
        let idx = index(text);
        let clamped = position_to_offset(text, &idx, Position::new(0, 99));
        assert_eq!(clamped, Some(TextSize::new(2)), "clamps before the newline");
        assert_eq!(position_to_offset(text, &idx, Position::new(9, 0)), None);
    }

    #[test]
    fn final_line_without_newline() {
        let text = "ab\ncd";
        let idx = index(text);
        assert_eq!(
            position_to_offset(text, &idx, Position::new(1, 2)),
            Some(TextSize::new(5))
        );
        assert_eq!(
            offset_to_position(text, &idx, TextSize::new(99)),
            Position::new(1, 2),
            "past-the-end offsets clamp"
        );
    }

    #[test]
    fn range_round_trip() {
        let text = "*start\nこんにちは\n";
        let idx = index(text);
        let range = TextRange::new(TextSize::new(7), TextSize::new(16)); // こんに
        let lsp = range_to_lsp(text, &idx, range);
        assert_eq!(lsp.start, Position::new(1, 0));
        assert_eq!(lsp.end, Position::new(1, 3));
    }
}
