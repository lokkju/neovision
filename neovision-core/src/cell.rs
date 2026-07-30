//! Character cell types matching VGA text-mode memory layout.
//!
//! A `Cell` is exactly 2 bytes: character byte + attribute byte.
//! The attribute byte matches VGA's native format (bits 0-3 fg, 4-6 bg, 7 blink),
//! which means `CellBuffer::as_bytes()` is directly comparable to a
//! captured 0xB8000 memory dump from a real VGA display.

#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Cell {
    pub ch: u8,
    pub attr: u8,
}

impl Cell {
    pub const BLANK: Cell = Cell {
        ch: b' ',
        attr: 0x07,
    }; // light gray on black

    pub const fn new(ch: u8, fg: u8, bg: u8, blink: bool) -> Cell {
        let attr = (fg & 0x0F) | ((bg & 0x07) << 4) | if blink { 0x80 } else { 0 };
        Cell { ch, attr }
    }

    pub const fn fg(self) -> u8 {
        self.attr & 0x0F
    }
    pub const fn bg(self) -> u8 {
        (self.attr >> 4) & 0x07
    }
    pub const fn blink(self) -> bool {
        (self.attr & 0x80) != 0
    }
}

use alloc::vec;
use alloc::vec::Vec;

/// A 2D grid of character cells with VGA byte layout.
///
/// Cells are stored row-major: index = row * cols + col.
#[derive(Debug, Clone)]
pub struct CellBuffer {
    pub cells: Vec<Cell>,
    pub cols: u16,
    pub rows: u16,
}

impl CellBuffer {
    /// Create a new buffer filled with `Cell::BLANK`.
    pub fn new(cols: u16, rows: u16) -> Self {
        Self {
            cells: vec![Cell::BLANK; cols as usize * rows as usize],
            cols,
            rows,
        }
    }

    /// Get a cell at (col, row). Panics if out of bounds.
    pub fn get(&self, col: u16, row: u16) -> Cell {
        self.cells[row as usize * self.cols as usize + col as usize]
    }

    /// Set a cell at (col, row). Panics if out of bounds.
    pub fn set(&mut self, col: u16, row: u16, cell: Cell) {
        self.cells[row as usize * self.cols as usize + col as usize] = cell;
    }

    /// Fill the entire buffer with a single cell value.
    pub fn fill(&mut self, cell: Cell) {
        self.cells.fill(cell);
    }

    /// Fill a single row with a cell value.
    pub fn fill_row(&mut self, row: u16, cell: Cell) {
        if row >= self.rows {
            return;
        }
        let start = row as usize * self.cols as usize;
        let end = start + self.cols as usize;
        self.cells[start..end].fill(cell);
    }

    /// Return the raw byte slice. Length is `cells.len() * 2`.
    ///
    /// The layout matches VGA text-mode memory exactly: char byte, attr byte,
    /// char byte, attr byte, ... row by row.
    pub fn as_bytes(&self) -> &[u8] {
        let ptr = self.cells.as_ptr() as *const u8;
        let len = self.cells.len() * 2;
        unsafe { core::slice::from_raw_parts(ptr, len) }
    }

    /// Shift all rows up by `n`. Exposed rows at the bottom are filled with BLANK.
    /// If `n >= rows`, clears the entire buffer.
    pub fn shift_up(&mut self, n: usize) {
        if n == 0 {
            return;
        }
        if n >= self.rows as usize {
            self.fill(Cell::BLANK);
            return;
        }
        let row_cells = self.cols as usize;
        let shift_cells = n * row_cells;
        self.cells.copy_within(shift_cells.., 0);
        let blank_start = self.cells.len() - shift_cells;
        self.cells[blank_start..].fill(Cell::BLANK);
    }

    /// Shift all rows down by `n`. Exposed rows at the top are filled with BLANK.
    /// If `n >= rows`, clears the entire buffer.
    pub fn shift_down(&mut self, n: usize) {
        if n == 0 {
            return;
        }
        if n >= self.rows as usize {
            self.fill(Cell::BLANK);
            return;
        }
        let row_cells = self.cols as usize;
        let shift_cells = n * row_cells;
        let keep_len = self.cells.len() - shift_cells;
        self.cells.copy_within(0..keep_len, shift_cells);
        self.cells[..shift_cells].fill(Cell::BLANK);
    }

    /// Write cells into a row. Writes up to `min(cells.len(), self.cols)` cells.
    /// No-op if `row >= self.rows`.
    pub fn write_row(&mut self, row: u16, cells: &[Cell]) {
        if row >= self.rows {
            return;
        }
        let start = row as usize * self.cols as usize;
        let count = cells.len().min(self.cols as usize);
        self.cells[start..start + count].copy_from_slice(&cells[..count]);
    }

    /// Resize the buffer, clearing content to BLANK.
    pub fn resize(&mut self, cols: u16, rows: u16) {
        self.cols = cols;
        self.rows = rows;
        self.cells.clear();
        self.cells
            .resize(cols as usize * rows as usize, Cell::BLANK);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cell_is_two_bytes() {
        assert_eq!(core::mem::size_of::<Cell>(), 2);
    }

    #[test]
    fn cell_new_packs_attr_correctly() {
        let c = Cell::new(b'A', 0xF, 0x1, true);
        assert_eq!(c.ch, b'A');
        assert_eq!(c.attr, 0x9F); // blink=1, bg=1, fg=F
        assert_eq!(c.fg(), 0xF);
        assert_eq!(c.bg(), 0x1);
        assert!(c.blink());
    }

    #[test]
    fn cell_blank_is_space_light_gray() {
        assert_eq!(Cell::BLANK.ch, b' ');
        assert_eq!(Cell::BLANK.fg(), 0x7);
        assert_eq!(Cell::BLANK.bg(), 0x0);
        assert!(!Cell::BLANK.blink());
    }

    #[test]
    fn buffer_new_correct_size() {
        let buf = CellBuffer::new(80, 25);
        assert_eq!(buf.cols, 80);
        assert_eq!(buf.rows, 25);
        assert_eq!(buf.cells.len(), 80 * 25);
        assert_eq!(buf.cells[0], Cell::BLANK);
    }

    #[test]
    fn buffer_get_set() {
        let mut buf = CellBuffer::new(4, 3);
        let c = Cell::new(b'X', 0xE, 0x1, false);
        buf.set(2, 1, c);
        assert_eq!(buf.get(2, 1), c);
        assert_eq!(buf.get(0, 0), Cell::BLANK);
    }

    #[test]
    fn buffer_fill_row_bounded() {
        let mut buf = CellBuffer::new(4, 3);
        let c = Cell::new(b'-', 0x7, 0, false);
        buf.fill_row(1, c);
        assert_eq!(buf.get(0, 0), Cell::BLANK);
        assert_eq!(buf.get(0, 1), c);
        assert_eq!(buf.get(3, 1), c);
        assert_eq!(buf.get(0, 2), Cell::BLANK);
        // Out-of-bounds row is a no-op
        buf.fill_row(99, c);
    }

    #[test]
    fn buffer_as_bytes_matches_vga_layout() {
        let mut buf = CellBuffer::new(2, 1);
        buf.set(
            0,
            0,
            Cell {
                ch: b'A',
                attr: 0x1F,
            },
        );
        buf.set(
            1,
            0,
            Cell {
                ch: b'B',
                attr: 0x2E,
            },
        );
        let bytes = buf.as_bytes();
        assert_eq!(bytes, &[b'A', 0x1F, b'B', 0x2E]);
    }

    fn fill_rows_with_chars(buf: &mut CellBuffer) {
        // Row r is filled with character b'0' + r
        for r in 0..buf.rows {
            for c in 0..buf.cols {
                buf.set(c, r, Cell::new(b'0' + r as u8, 0x7, 0, false));
            }
        }
    }

    #[test]
    fn shift_up_moves_rows_and_blanks_bottom() {
        let mut buf = CellBuffer::new(3, 4);
        fill_rows_with_chars(&mut buf);
        buf.shift_up(1);
        assert_eq!(buf.get(0, 0).ch, b'1');
        assert_eq!(buf.get(0, 1).ch, b'2');
        assert_eq!(buf.get(0, 2).ch, b'3');
        assert_eq!(buf.get(0, 3), Cell::BLANK);
    }

    #[test]
    fn shift_up_multiple_rows() {
        let mut buf = CellBuffer::new(2, 5);
        fill_rows_with_chars(&mut buf);
        buf.shift_up(3);
        assert_eq!(buf.get(0, 0).ch, b'3');
        assert_eq!(buf.get(0, 1).ch, b'4');
        assert_eq!(buf.get(0, 2), Cell::BLANK);
        assert_eq!(buf.get(0, 3), Cell::BLANK);
        assert_eq!(buf.get(0, 4), Cell::BLANK);
    }

    #[test]
    fn shift_up_beyond_rows_clears_all() {
        let mut buf = CellBuffer::new(2, 3);
        fill_rows_with_chars(&mut buf);
        buf.shift_up(5);
        for r in 0..3 {
            assert_eq!(buf.get(0, r), Cell::BLANK);
        }
    }

    #[test]
    fn shift_down_moves_rows_and_blanks_top() {
        let mut buf = CellBuffer::new(3, 4);
        fill_rows_with_chars(&mut buf);
        buf.shift_down(1);
        assert_eq!(buf.get(0, 0), Cell::BLANK);
        assert_eq!(buf.get(0, 1).ch, b'0');
        assert_eq!(buf.get(0, 2).ch, b'1');
        assert_eq!(buf.get(0, 3).ch, b'2');
    }

    #[test]
    fn write_row_copies_cells() {
        let mut buf = CellBuffer::new(4, 2);
        let row = [
            Cell::new(b'A', 0, 0, false),
            Cell::new(b'B', 0, 0, false),
            Cell::new(b'C', 0, 0, false),
        ];
        buf.write_row(1, &row);
        assert_eq!(buf.get(0, 1).ch, b'A');
        assert_eq!(buf.get(1, 1).ch, b'B');
        assert_eq!(buf.get(2, 1).ch, b'C');
        assert_eq!(buf.get(3, 1), Cell::BLANK);
    }

    #[test]
    fn write_row_truncates_oversize() {
        let mut buf = CellBuffer::new(2, 1);
        let row = [
            Cell::new(b'A', 0, 0, false),
            Cell::new(b'B', 0, 0, false),
            Cell::new(b'C', 0, 0, false),
        ];
        buf.write_row(0, &row);
        assert_eq!(buf.get(0, 0).ch, b'A');
        assert_eq!(buf.get(1, 0).ch, b'B');
    }

    #[test]
    fn resize_clears_and_resizes() {
        let mut buf = CellBuffer::new(3, 3);
        fill_rows_with_chars(&mut buf);
        buf.resize(4, 2);
        assert_eq!(buf.cols, 4);
        assert_eq!(buf.rows, 2);
        assert_eq!(buf.cells.len(), 8);
        for c in &buf.cells {
            assert_eq!(*c, Cell::BLANK);
        }
    }
}
