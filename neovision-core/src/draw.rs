//! A small drawing vocabulary shared by [`CellBuffer`] and [`Layer`].
//!
//! [`CellCanvas`] is the minimal surface a target must provide; [`CellDraw`]
//! is blanket-implemented on top of it, so both targets get every helper for
//! free. Text is CP437 bytes, not Unicode.

use super::cell::{Cell, CellBuffer};
use super::cp437;
use super::geom::{Point, Rect, Size};
use super::layer::{Layer, LayerCell};

/// A surface that cells can be written to.
///
/// `put` is bounds-safe by contract — writing outside the surface is a no-op,
/// never a panic. That is what lets callers draw a panel that overhangs an
/// edge without pre-clipping.
pub trait CellCanvas {
    /// Dimensions of this surface, in its own local coordinates.
    fn size(&self) -> Size;

    /// Write one cell. Out of bounds is a silent no-op.
    fn put(&mut self, x: u16, y: u16, cell: Cell);
}

impl CellCanvas for CellBuffer {
    fn size(&self) -> Size {
        Size::new(self.cols, self.rows)
    }

    fn put(&mut self, x: u16, y: u16, cell: Cell) {
        // CellBuffer::set panics out of bounds; guard it here.
        if x >= self.cols || y >= self.rows {
            return;
        }
        self.set(x, y, cell);
    }
}

impl CellCanvas for Layer {
    fn size(&self) -> Size {
        self.bounds().size
    }

    fn put(&mut self, x: u16, y: u16, cell: Cell) {
        // Layer::set is already bounds-safe.
        self.set(x, y, LayerCell::Opaque(cell));
    }
}

/// The CP437 glyphs that make up a box frame, including the tee junctions
/// used where an interior separator meets the frame.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BoxChars {
    pub tl: u8,
    pub tr: u8,
    pub bl: u8,
    pub br: u8,
    pub h: u8,
    pub v: u8,
    /// Tee opening rightward from the left wall (`├`) — start of a
    /// horizontal separator inside a framed panel.
    pub tee_l: u8,
    /// Tee opening leftward from the right wall (`┤`) — end of a
    /// horizontal separator inside a framed panel.
    pub tee_r: u8,
    /// Tee opening downward from the top wall (`┬`).
    pub tee_t: u8,
    /// Tee opening upward from the bottom wall (`┴`).
    pub tee_b: u8,
}

impl BoxChars {
    /// CP437 single-line frame — CUA uses this for inner rules.
    pub const SINGLE: BoxChars = BoxChars {
        tl: 0xDA,
        tr: 0xBF,
        bl: 0xC0,
        br: 0xD9,
        h: 0xC4,
        v: 0xB3,
        tee_l: 0xC3,
        tee_r: 0xB4,
        tee_t: 0xC2,
        tee_b: 0xC1,
    };

    /// CP437 double-line frame — CUA uses this for dialog borders.
    pub const DOUBLE: BoxChars = BoxChars {
        tl: 0xC9,
        tr: 0xBB,
        bl: 0xC8,
        br: 0xBC,
        h: 0xCD,
        v: 0xBA,
        tee_l: 0xCC,
        tee_r: 0xB9,
        tee_t: 0xCB,
        tee_b: 0xCA,
    };
}

/// Drawing helpers, blanket-implemented for every [`CellCanvas`].
pub trait CellDraw: CellCanvas {
    /// Write `s` starting at `at`, truncating at the right edge.
    ///
    /// Returns the number of cells actually written — one per `char`, so the
    /// count is the rendered width, not a byte length.
    ///
    /// Characters are folded to CP437 through [`cp437::from_char`]; anything
    /// CP437 cannot represent becomes `?`. One `char` always occupies exactly
    /// one cell, which is what lets callers reason about column alignment.
    fn write_str(&mut self, at: Point, s: &str, attr: u8) -> u16 {
        let size = self.size();
        if at.y >= size.h {
            return 0;
        }
        let mut x = at.x;
        let mut written = 0u16;
        for ch in s.chars() {
            if x >= size.w {
                break;
            }
            let byte = cp437::from_char(ch).unwrap_or(b'?');
            self.put(x, at.y, Cell { ch: byte, attr });
            x = x.saturating_add(1);
            written = written.saturating_add(1);
        }
        written
    }

    /// Fill `r` with `cell`, clipped to the canvas.
    fn fill_rect(&mut self, r: Rect, cell: Cell) {
        let size = self.size();
        let canvas = Rect::new(0, 0, size.w, size.h);
        let Some(clip) = r.intersect(&canvas) else {
            return;
        };
        for y in clip.top()..clip.bottom() {
            for x in clip.left()..clip.right() {
                self.put(x, y, cell);
            }
        }
    }

    /// Draw a horizontal run of `ch`, clipped to the canvas.
    fn draw_hline(&mut self, at: Point, len: u16, ch: u8, attr: u8) {
        for i in 0..len {
            let Some(x) = at.x.checked_add(i) else {
                break;
            };
            self.put(x, at.y, Cell { ch, attr });
        }
    }

    /// Draw a box frame around `r`. The interior is left untouched — call
    /// `fill_rect` first for a filled panel.
    ///
    /// Rects narrower or shorter than 2 collapse gracefully: corners are
    /// drawn last, so a 1x1 rect ends up as a single corner glyph rather
    /// than a panic.
    fn draw_box(&mut self, r: Rect, chars: BoxChars, attr: u8) {
        if r.size.is_empty() {
            return;
        }
        let x0 = r.left();
        let y0 = r.top();
        let x1 = r.right() - 1; // inclusive
        let y1 = r.bottom() - 1;

        if r.size.w > 2 {
            for x in x0.saturating_add(1)..x1 {
                self.put(x, y0, Cell { ch: chars.h, attr });
                self.put(x, y1, Cell { ch: chars.h, attr });
            }
        }
        if r.size.h > 2 {
            for y in y0.saturating_add(1)..y1 {
                self.put(x0, y, Cell { ch: chars.v, attr });
                self.put(x1, y, Cell { ch: chars.v, attr });
            }
        }

        // Corners last so they win on degenerate rects.
        self.put(x0, y0, Cell { ch: chars.tl, attr });
        self.put(x1, y0, Cell { ch: chars.tr, attr });
        self.put(x0, y1, Cell { ch: chars.bl, attr });
        self.put(x1, y1, Cell { ch: chars.br, attr });
    }
}

impl<T: CellCanvas + ?Sized> CellDraw for T {}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::layer::{Layer, LayerCell};

    const ATTR: u8 = 0x1F;

    #[test]
    fn write_str_writes_chars_and_reports_count() {
        let mut buf = CellBuffer::new(8, 2);
        let n = buf.write_str(Point::new(1, 0), "Hi", ATTR);
        assert_eq!(n, 2);
        assert_eq!(buf.get(1, 0).ch, b'H');
        assert_eq!(buf.get(1, 0).attr, ATTR);
        assert_eq!(buf.get(2, 0).ch, b'i');
        assert_eq!(buf.get(0, 0), Cell::BLANK);
    }

    #[test]
    fn write_str_truncates_at_the_right_edge() {
        let mut buf = CellBuffer::new(4, 1);
        let n = buf.write_str(Point::new(2, 0), "ABCD", ATTR);
        assert_eq!(n, 2);
        assert_eq!(buf.get(2, 0).ch, b'A');
        assert_eq!(buf.get(3, 0).ch, b'B');
    }

    #[test]
    fn write_str_below_the_canvas_writes_nothing() {
        let mut buf = CellBuffer::new(4, 2);
        assert_eq!(buf.write_str(Point::new(0, 5), "ABC", ATTR), 0);
        assert_eq!(buf.get(0, 0), Cell::BLANK);
    }

    #[test]
    fn write_str_starting_beyond_the_right_edge_writes_nothing() {
        let mut buf = CellBuffer::new(4, 2);
        assert_eq!(buf.write_str(Point::new(4, 0), "ABC", ATTR), 0);
        assert_eq!(buf.write_str(Point::new(99, 0), "ABC", ATTR), 0);
        for x in 0..4 {
            assert_eq!(buf.get(x, 0), Cell::BLANK);
        }
    }

    #[test]
    fn write_str_folds_non_ascii_into_its_cp437_byte() {
        let mut buf = CellBuffer::new(4, 1);
        let n = buf.write_str(Point::new(0, 0), "aé", ATTR);
        assert_eq!(n, 2);
        assert_eq!(buf.get(0, 0).ch, b'a');
        assert_eq!(buf.get(1, 0).ch, 0x82); // 'é' is in the CP437 repertoire
    }

    #[test]
    fn write_str_maps_chars_outside_cp437_to_question_mark() {
        let mut buf = CellBuffer::new(4, 1);
        // An em dash has no CP437 byte, unlike the box glyphs above it.
        let n = buf.write_str(Point::new(0, 0), "a—│", ATTR);
        assert_eq!(n, 3);
        assert_eq!(buf.get(0, 0).ch, b'a');
        assert_eq!(buf.get(1, 0).ch, b'?');
        assert_eq!(buf.get(2, 0).ch, 0xB3);
    }

    #[test]
    fn write_str_writes_one_cell_per_char_regardless_of_utf8_width() {
        let mut buf = CellBuffer::new(8, 1);
        // Three chars, five UTF-8 bytes.
        assert_eq!(buf.write_str(Point::new(0, 0), "½¼x", ATTR), 3);
        assert_eq!(buf.get(0, 0).ch, 0xAB);
        assert_eq!(buf.get(1, 0).ch, 0xAC);
        assert_eq!(buf.get(2, 0).ch, b'x');
        assert_eq!(buf.get(3, 0), Cell::BLANK);
    }

    #[test]
    fn fill_rect_fills_only_the_requested_region() {
        let mut buf = CellBuffer::new(4, 3);
        let c = Cell::new(b'#', 0x7, 0x0, false);
        buf.fill_rect(Rect::new(1, 1, 2, 1), c);
        assert_eq!(buf.get(1, 1), c);
        assert_eq!(buf.get(2, 1), c);
        assert_eq!(buf.get(0, 1), Cell::BLANK);
        assert_eq!(buf.get(3, 1), Cell::BLANK);
        assert_eq!(buf.get(1, 0), Cell::BLANK);
    }

    #[test]
    fn fill_rect_clips_against_the_canvas() {
        let mut buf = CellBuffer::new(2, 2);
        let c = Cell::new(b'#', 0x7, 0x0, false);
        buf.fill_rect(Rect::new(1, 1, 9, 9), c);
        assert_eq!(buf.get(1, 1), c);
        assert_eq!(buf.get(0, 0), Cell::BLANK);
    }

    #[test]
    fn drawing_into_a_layer_produces_opaque_cells() {
        let mut l = Layer::new(Point::new(5, 5), Size::new(4, 1));
        let n = l.write_str(Point::new(0, 0), "ok", ATTR);
        assert_eq!(n, 2);
        assert_eq!(
            l.get(0, 0),
            LayerCell::Opaque(Cell {
                ch: b'o',
                attr: ATTR
            })
        );
        assert_eq!(l.get(2, 0), LayerCell::Transparent);
    }

    #[test]
    fn layer_canvas_size_is_layer_local_not_screen_relative() {
        let l = Layer::new(Point::new(70, 20), Size::new(4, 2));
        assert_eq!(CellCanvas::size(&l), Size::new(4, 2));
    }

    #[test]
    fn draw_box_places_double_line_corners_and_edges() {
        let mut buf = CellBuffer::new(3, 3);
        buf.draw_box(Rect::new(0, 0, 3, 3), BoxChars::DOUBLE, ATTR);
        assert_eq!(buf.get(0, 0).ch, 0xC9); // top-left
        assert_eq!(buf.get(2, 0).ch, 0xBB); // top-right
        assert_eq!(buf.get(0, 2).ch, 0xC8); // bottom-left
        assert_eq!(buf.get(2, 2).ch, 0xBC); // bottom-right
        assert_eq!(buf.get(1, 0).ch, 0xCD); // top edge
        assert_eq!(buf.get(1, 2).ch, 0xCD); // bottom edge
        assert_eq!(buf.get(0, 1).ch, 0xBA); // left edge
        assert_eq!(buf.get(2, 1).ch, 0xBA); // right edge
        assert_eq!(buf.get(1, 1), Cell::BLANK); // interior untouched
        assert_eq!(buf.get(0, 0).attr, ATTR);
    }

    #[test]
    fn draw_box_single_line_uses_single_glyphs() {
        let mut buf = CellBuffer::new(3, 3);
        buf.draw_box(Rect::new(0, 0, 3, 3), BoxChars::SINGLE, ATTR);
        assert_eq!(buf.get(0, 0).ch, 0xDA);
        assert_eq!(buf.get(2, 0).ch, 0xBF);
        assert_eq!(buf.get(0, 2).ch, 0xC0);
        assert_eq!(buf.get(2, 2).ch, 0xD9);
        assert_eq!(buf.get(1, 0).ch, 0xC4);
        assert_eq!(buf.get(0, 1).ch, 0xB3);
    }

    #[test]
    fn draw_box_on_two_by_two_is_all_corners() {
        let mut buf = CellBuffer::new(2, 2);
        buf.draw_box(Rect::new(0, 0, 2, 2), BoxChars::DOUBLE, ATTR);
        assert_eq!(buf.get(0, 0).ch, 0xC9);
        assert_eq!(buf.get(1, 0).ch, 0xBB);
        assert_eq!(buf.get(0, 1).ch, 0xC8);
        assert_eq!(buf.get(1, 1).ch, 0xBC);
    }

    #[test]
    fn draw_box_on_degenerate_rects_does_not_panic() {
        let mut buf = CellBuffer::new(4, 4);
        buf.draw_box(Rect::new(0, 0, 1, 1), BoxChars::DOUBLE, ATTR);
        buf.draw_box(Rect::new(1, 1, 0, 0), BoxChars::DOUBLE, ATTR);
        buf.draw_box(Rect::new(2, 2, 1, 3), BoxChars::DOUBLE, ATTR);
        // A 1x1 box collapses to a single cell; the last corner drawn wins.
        assert_eq!(buf.get(0, 0).ch, 0xBC);
    }

    #[test]
    fn draw_box_clips_when_it_overhangs_the_canvas() {
        let mut buf = CellBuffer::new(3, 3);
        buf.draw_box(Rect::new(1, 1, 9, 9), BoxChars::DOUBLE, ATTR);
        assert_eq!(buf.get(1, 1).ch, 0xC9);
        assert_eq!(buf.get(0, 0), Cell::BLANK);
    }

    #[test]
    fn box_chars_tee_glyphs_are_the_correct_cp437_bytes() {
        assert_eq!(BoxChars::SINGLE.tee_l, 0xC3);
        assert_eq!(BoxChars::SINGLE.tee_r, 0xB4);
        assert_eq!(BoxChars::SINGLE.tee_t, 0xC2);
        assert_eq!(BoxChars::SINGLE.tee_b, 0xC1);
        assert_eq!(BoxChars::DOUBLE.tee_l, 0xCC);
        assert_eq!(BoxChars::DOUBLE.tee_r, 0xB9);
        assert_eq!(BoxChars::DOUBLE.tee_t, 0xCB);
        assert_eq!(BoxChars::DOUBLE.tee_b, 0xCA);
    }

    #[test]
    fn draw_hline_draws_a_run_and_clips() {
        let mut buf = CellBuffer::new(4, 2);
        buf.draw_hline(Point::new(1, 1), 9, 0xC4, ATTR);
        assert_eq!(buf.get(0, 1), Cell::BLANK);
        assert_eq!(buf.get(1, 1).ch, 0xC4);
        assert_eq!(buf.get(3, 1).ch, 0xC4);
        assert_eq!(buf.get(1, 1).attr, ATTR);
    }

    #[test]
    fn draw_hline_zero_length_writes_nothing() {
        let mut buf = CellBuffer::new(4, 1);
        buf.draw_hline(Point::new(0, 0), 0, 0xC4, ATTR);
        assert_eq!(buf.get(0, 0), Cell::BLANK);
    }

    #[test]
    fn draw_box_at_the_coordinate_limit_does_not_overflow() {
        let mut buf = CellBuffer::new(4, 4);
        // origin.x == u16::MAX with w > 2 used to overflow `x0 + 1`.
        buf.draw_box(Rect::new(u16::MAX, 0, 3, 1), BoxChars::DOUBLE, ATTR);
        // origin.y == u16::MAX with h > 2 used to overflow `y0 + 1`.
        buf.draw_box(Rect::new(0, u16::MAX, 1, 3), BoxChars::DOUBLE, ATTR);
        buf.draw_box(Rect::new(u16::MAX, u16::MAX, 5, 5), BoxChars::DOUBLE, ATTR);
        // All three rects are entirely offscreen, so nothing was drawn.
        for y in 0..4 {
            for x in 0..4 {
                assert_eq!(buf.get(x, y), Cell::BLANK);
            }
        }
    }
}
