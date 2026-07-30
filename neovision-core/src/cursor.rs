//! A text-cursor descriptor: a screen-cell position + shape that a host
//! realizes (pixel scanlines, or a future text-mode BIOS cursor). It is never
//! written into the cell buffer, so it never overwrites a glyph.

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CursorShape {
    /// Thin underline at the bottom of the cell.
    Insert,
    /// Full-cell block (reverse video).
    Overtype,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TextCursor {
    pub col: u16,
    pub row: u16,
    pub shape: CursorShape,
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn cursor_carries_position_and_shape() {
        let c = TextCursor {
            col: 5,
            row: 2,
            shape: CursorShape::Insert,
        };
        assert_eq!((c.col, c.row), (5, 2));
        assert_eq!(c.shape, CursorShape::Insert);
    }
}
