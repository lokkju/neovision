//! Rectangular layers stacked over a base [`CellBuffer`].
//!
//! This is Turbo Vision's view model: each layer is a rectangle at an origin,
//! and the stack is flattened onto the base at display time. The scroll owns
//! the base buffer and shifts rows out of it, so anything drawn directly there
//! is destroyed — layers are how an overlay survives.

use alloc::vec;
use alloc::vec::Vec;

use super::cell::Cell;
use super::geom::{Point, Rect, Size};

/// The attribute a [`LayerCell::Shade`] forces onto the cell beneath it:
/// dark grey (8) on black (0), no blink. This is what Turbo Vision's drop
/// shadow did.
pub const SHADE_ATTR: u8 = 0x08;

/// One cell of a layer.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum LayerCell {
    /// The underlying cell shows through unchanged.
    #[default]
    Transparent,
    /// Replaces the underlying cell entirely.
    Opaque(Cell),
    /// Keeps the underlying character but forces [`SHADE_ATTR`].
    ///
    /// Needed because a drop shadow is L-shaped: a panel's bounding rect,
    /// once grown to include its shadow, holds cells that are neither.
    Shade,
}

/// A rectangular region of [`LayerCell`]s at an origin on the base grid.
#[derive(Debug, Clone)]
pub struct Layer {
    /// Position of this layer's top-left cell on the base grid.
    pub origin: Point,
    size: Size,
    cells: Vec<LayerCell>,
}

impl Layer {
    /// A fully transparent layer.
    pub fn new(origin: Point, size: Size) -> Self {
        Self {
            origin,
            size,
            cells: vec![LayerCell::Transparent; size.w as usize * size.h as usize],
        }
    }

    /// A layer filled entirely with one opaque cell.
    pub fn filled(origin: Point, size: Size, cell: Cell) -> Self {
        Self {
            origin,
            size,
            cells: vec![LayerCell::Opaque(cell); size.w as usize * size.h as usize],
        }
    }

    pub fn size(&self) -> Size {
        self.size
    }

    /// This layer's rect on the base grid.
    ///
    /// This is in SCREEN coordinates (`origin` included). To fill or draw
    /// into the layer itself, use [`Layer::local_bounds`] instead — the
    /// drawing methods (`CellDraw`, `Layer::fill`) all operate in
    /// layer-local space.
    pub fn bounds(&self) -> Rect {
        Rect {
            origin: self.origin,
            size: self.size,
        }
    }

    /// This layer's rect in its OWN local coordinates, origin at (0, 0).
    ///
    /// Use this, not [`Layer::bounds`], when passing a rect to the drawing
    /// methods — those work in layer-local space, while `bounds()` is the
    /// layer's position on the base grid.
    pub fn local_bounds(&self) -> Rect {
        Rect::new(0, 0, self.size.w, self.size.h)
    }

    /// Read a cell in layer-local coordinates. Out of bounds reads as
    /// `Transparent` rather than panicking.
    pub fn get(&self, x: u16, y: u16) -> LayerCell {
        if x >= self.size.w || y >= self.size.h {
            return LayerCell::Transparent;
        }
        self.cells[y as usize * self.size.w as usize + x as usize]
    }

    /// Write a cell in layer-local coordinates. Out of bounds is a no-op.
    pub fn set(&mut self, x: u16, y: u16, c: LayerCell) {
        if x >= self.size.w || y >= self.size.h {
            return;
        }
        self.cells[y as usize * self.size.w as usize + x as usize] = c;
    }

    /// Fill a layer-local region with any [`LayerCell`], clipped to this
    /// layer's bounds.
    ///
    /// `CellDraw::fill_rect` can only write `Opaque` cells; this is how a
    /// caller paints `Shade` (drop shadows) or punches a region back to
    /// `Transparent`.
    pub fn fill(&mut self, r: Rect, c: LayerCell) {
        let local = Rect::new(0, 0, self.size.w, self.size.h);
        let Some(clip) = r.intersect(&local) else {
            return;
        };
        for y in clip.top()..clip.bottom() {
            for x in clip.left()..clip.right() {
                self.set(x, y, c);
            }
        }
    }
}

use super::cell::CellBuffer;

/// A bottom-to-top stack of layers, flattened onto a base buffer.
#[derive(Debug, Clone, Default)]
pub struct LayerStack {
    layers: Vec<Layer>,
}

impl LayerStack {
    pub fn new() -> Self {
        Self::default()
    }

    /// Push a layer on top of the stack.
    pub fn push(&mut self, layer: Layer) {
        self.layers.push(layer);
    }

    /// Remove and return the topmost layer.
    pub fn pop(&mut self) -> Option<Layer> {
        self.layers.pop()
    }

    /// Mutable access to the topmost layer.
    pub fn top_mut(&mut self) -> Option<&mut Layer> {
        self.layers.last_mut()
    }

    /// Mutable access by index, 0 being the bottom layer.
    pub fn get_mut(&mut self, index: usize) -> Option<&mut Layer> {
        self.layers.get_mut(index)
    }

    /// Read-only access by index, 0 being the bottom layer.
    pub fn get(&self, index: usize) -> Option<&Layer> {
        self.layers.get(index)
    }

    /// Iterate layers bottom-to-top.
    pub fn iter(&self) -> impl Iterator<Item = &Layer> {
        self.layers.iter()
    }

    pub fn len(&self) -> usize {
        self.layers.len()
    }

    pub fn is_empty(&self) -> bool {
        self.layers.is_empty()
    }

    pub fn clear(&mut self) {
        self.layers.clear();
    }

    /// Flatten `base` plus every layer into `out`.
    ///
    /// `out` is resized to match `base` when their dimensions differ. Layers
    /// are applied bottom-to-top, each clipped to the base's bounds, so an
    /// oversized or offscreen layer is truncated rather than panicking.
    pub fn composite(&self, base: &CellBuffer, out: &mut CellBuffer) {
        if out.cols != base.cols || out.rows != base.rows {
            out.resize(base.cols, base.rows);
        }
        out.cells.copy_from_slice(&base.cells);

        let screen = Rect::new(0, 0, base.cols, base.rows);
        for layer in &self.layers {
            let Some(clip) = layer.bounds().intersect(&screen) else {
                continue;
            };
            for y in clip.top()..clip.bottom() {
                for x in clip.left()..clip.right() {
                    // `clip` is inside `layer.bounds()`, so these cannot underflow.
                    let lx = x - layer.origin.x;
                    let ly = y - layer.origin.y;
                    match layer.get(lx, ly) {
                        LayerCell::Transparent => {}
                        LayerCell::Opaque(c) => out.set(x, y, c),
                        LayerCell::Shade => {
                            let mut c = out.get(x, y);
                            c.attr = SHADE_ATTR;
                            out.set(x, y, c);
                        }
                    }
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cell(ch: u8) -> Cell {
        Cell::new(ch, 0x7, 0x0, false)
    }

    #[test]
    fn new_layer_is_all_transparent() {
        let l = Layer::new(Point::new(0, 0), Size::new(3, 2));
        for y in 0..2 {
            for x in 0..3 {
                assert_eq!(l.get(x, y), LayerCell::Transparent);
            }
        }
    }

    #[test]
    fn filled_layer_is_all_opaque() {
        let c = cell(b'#');
        let l = Layer::filled(Point::new(1, 1), Size::new(2, 2), c);
        assert_eq!(l.get(0, 0), LayerCell::Opaque(c));
        assert_eq!(l.get(1, 1), LayerCell::Opaque(c));
    }

    #[test]
    fn set_get_round_trip_in_layer_local_coords() {
        let mut l = Layer::new(Point::new(10, 20), Size::new(4, 3));
        let c = cell(b'X');
        l.set(2, 1, LayerCell::Opaque(c));
        assert_eq!(l.get(2, 1), LayerCell::Opaque(c));
        // Origin does not shift local addressing.
        assert_eq!(l.get(0, 0), LayerCell::Transparent);
    }

    #[test]
    fn set_out_of_bounds_is_a_silent_noop() {
        let mut l = Layer::new(Point::new(0, 0), Size::new(2, 2));
        l.set(2, 0, LayerCell::Shade);
        l.set(0, 2, LayerCell::Shade);
        l.set(99, 99, LayerCell::Shade);
        // Nothing was written and nothing panicked.
        assert_eq!(l.get(0, 0), LayerCell::Transparent);
        assert_eq!(l.get(1, 1), LayerCell::Transparent);
    }

    #[test]
    fn get_out_of_bounds_returns_transparent() {
        let l = Layer::filled(Point::new(0, 0), Size::new(2, 2), cell(b'#'));
        assert_eq!(l.get(5, 5), LayerCell::Transparent);
    }

    #[test]
    fn bounds_combines_origin_and_size() {
        let l = Layer::new(Point::new(7, 9), Size::new(4, 5));
        assert_eq!(l.bounds(), Rect::new(7, 9, 4, 5));
        assert_eq!(l.size(), Size::new(4, 5));
    }

    #[test]
    fn shade_attr_is_dark_grey_on_black() {
        assert_eq!(SHADE_ATTR, 0x08);
        let c = Cell {
            ch: b'A',
            attr: SHADE_ATTR,
        };
        assert_eq!(c.fg(), 0x8);
        assert_eq!(c.bg(), 0x0);
        assert!(!c.blink());
    }

    #[test]
    fn indexing_is_row_major_and_not_transposed() {
        // 4 wide, 2 tall — deliberately non-square so a transposed index
        // formula lands on the wrong cell or out of bounds.
        let mut l = Layer::new(Point::new(0, 0), Size::new(4, 2));
        l.set(3, 0, LayerCell::Opaque(cell(b'A')));
        l.set(0, 1, LayerCell::Opaque(cell(b'B')));

        assert_eq!(l.get(3, 0), LayerCell::Opaque(cell(b'A')));
        assert_eq!(l.get(0, 1), LayerCell::Opaque(cell(b'B')));
        // The transposed positions must be untouched.
        assert_eq!(l.get(0, 0), LayerCell::Transparent);
        assert_eq!(l.get(1, 0), LayerCell::Transparent);
    }

    use crate::cell::CellBuffer;

    fn base_filled(cols: u16, rows: u16, ch: u8) -> CellBuffer {
        let mut b = CellBuffer::new(cols, rows);
        for y in 0..rows {
            for x in 0..cols {
                b.set(x, y, Cell::new(ch, 0x7, 0x0, false));
            }
        }
        b
    }

    #[test]
    fn empty_stack_composites_to_an_exact_copy_of_base() {
        let base = base_filled(4, 3, b'.');
        let mut out = CellBuffer::new(4, 3);
        LayerStack::new().composite(&base, &mut out);
        assert_eq!(out.cells, base.cells);
    }

    #[test]
    fn opaque_overwrites_the_base_cell() {
        let base = base_filled(4, 3, b'.');
        let mut out = CellBuffer::new(4, 3);
        let mut stack = LayerStack::new();
        stack.push(Layer::filled(Point::new(1, 1), Size::new(2, 1), cell(b'#')));
        stack.composite(&base, &mut out);
        assert_eq!(out.get(1, 1).ch, b'#');
        assert_eq!(out.get(2, 1).ch, b'#');
        assert_eq!(out.get(0, 1).ch, b'.');
        assert_eq!(out.get(3, 1).ch, b'.');
    }

    #[test]
    fn transparent_leaves_the_base_cell_untouched() {
        let base = base_filled(3, 3, b'.');
        let mut out = CellBuffer::new(3, 3);
        let mut stack = LayerStack::new();
        // A wholly transparent layer covering everything.
        stack.push(Layer::new(Point::new(0, 0), Size::new(3, 3)));
        stack.composite(&base, &mut out);
        assert_eq!(out.cells, base.cells);
    }

    #[test]
    fn shade_preserves_char_and_forces_shade_attr() {
        let base = base_filled(3, 1, b'A');
        let mut out = CellBuffer::new(3, 1);
        let mut stack = LayerStack::new();
        let mut l = Layer::new(Point::new(1, 0), Size::new(1, 1));
        l.set(0, 0, LayerCell::Shade);
        stack.push(l);
        stack.composite(&base, &mut out);
        assert_eq!(out.get(1, 0).ch, b'A');
        assert_eq!(out.get(1, 0).attr, SHADE_ATTR);
        // Neighbours untouched.
        assert_eq!(out.get(0, 0).attr, base.get(0, 0).attr);
    }

    #[test]
    fn upper_layer_wins_where_layers_overlap() {
        let base = base_filled(4, 1, b'.');
        let mut out = CellBuffer::new(4, 1);
        let mut stack = LayerStack::new();
        stack.push(Layer::filled(Point::new(0, 0), Size::new(3, 1), cell(b'L')));
        stack.push(Layer::filled(Point::new(2, 0), Size::new(2, 1), cell(b'U')));
        stack.composite(&base, &mut out);
        assert_eq!(out.get(0, 0).ch, b'L');
        assert_eq!(out.get(1, 0).ch, b'L');
        assert_eq!(out.get(2, 0).ch, b'U'); // overlap: upper wins
        assert_eq!(out.get(3, 0).ch, b'U');
    }

    #[test]
    fn layer_overhanging_right_edge_clips_instead_of_panicking() {
        let base = base_filled(3, 1, b'.');
        let mut out = CellBuffer::new(3, 1);
        let mut stack = LayerStack::new();
        stack.push(Layer::filled(Point::new(2, 0), Size::new(5, 1), cell(b'#')));
        stack.composite(&base, &mut out);
        assert_eq!(out.get(2, 0).ch, b'#');
        assert_eq!(out.get(0, 0).ch, b'.');
    }

    #[test]
    fn layer_overhanging_bottom_edge_clips_instead_of_panicking() {
        let base = base_filled(1, 3, b'.');
        let mut out = CellBuffer::new(1, 3);
        let mut stack = LayerStack::new();
        stack.push(Layer::filled(Point::new(0, 2), Size::new(1, 9), cell(b'#')));
        stack.composite(&base, &mut out);
        assert_eq!(out.get(0, 2).ch, b'#');
        assert_eq!(out.get(0, 0).ch, b'.');
    }

    #[test]
    fn layer_fully_offscreen_is_skipped() {
        let base = base_filled(2, 2, b'.');
        let mut out = CellBuffer::new(2, 2);
        let mut stack = LayerStack::new();
        stack.push(Layer::filled(Point::new(9, 9), Size::new(2, 2), cell(b'#')));
        stack.composite(&base, &mut out);
        assert_eq!(out.cells, base.cells);
    }

    #[test]
    fn composite_resizes_a_mismatched_output_buffer() {
        let base = base_filled(4, 2, b'.');
        let mut out = CellBuffer::new(1, 1);
        LayerStack::new().composite(&base, &mut out);
        assert_eq!(out.cols, 4);
        assert_eq!(out.rows, 2);
        assert_eq!(out.cells, base.cells);
    }

    #[test]
    fn stack_push_pop_and_len() {
        let mut stack = LayerStack::new();
        assert!(stack.is_empty());
        stack.push(Layer::new(Point::new(0, 0), Size::new(1, 1)));
        stack.push(Layer::new(Point::new(1, 1), Size::new(1, 1)));
        assert_eq!(stack.len(), 2);
        let popped = stack.pop().expect("layer");
        assert_eq!(popped.origin, Point::new(1, 1));
        assert_eq!(stack.len(), 1);
        stack.clear();
        assert!(stack.is_empty());
        assert!(stack.pop().is_none());
    }

    #[test]
    fn top_mut_addresses_the_upper_layer() {
        let mut stack = LayerStack::new();
        stack.push(Layer::new(Point::new(0, 0), Size::new(2, 1)));
        stack.push(Layer::new(Point::new(0, 0), Size::new(2, 1)));
        stack
            .top_mut()
            .expect("layer")
            .set(0, 0, LayerCell::Opaque(cell(b'T')));
        assert_eq!(
            stack.get_mut(1).expect("layer").get(0, 0),
            LayerCell::Opaque(cell(b'T'))
        );
        assert_eq!(
            stack.get_mut(0).expect("layer").get(0, 0),
            LayerCell::Transparent
        );
    }

    #[test]
    fn fill_writes_the_requested_region_and_leaves_the_rest_untouched() {
        let mut l = Layer::new(Point::new(3, 3), Size::new(4, 4));
        l.fill(Rect::new(1, 1, 2, 2), LayerCell::Shade);
        assert_eq!(l.get(1, 1), LayerCell::Shade);
        assert_eq!(l.get(2, 1), LayerCell::Shade);
        assert_eq!(l.get(1, 2), LayerCell::Shade);
        assert_eq!(l.get(2, 2), LayerCell::Shade);
        // Outside the filled rect is untouched.
        assert_eq!(l.get(0, 0), LayerCell::Transparent);
        assert_eq!(l.get(3, 3), LayerCell::Transparent);
        assert_eq!(l.get(0, 1), LayerCell::Transparent);
    }

    #[test]
    fn fill_clips_an_oversized_rect_without_panicking() {
        let mut l = Layer::new(Point::new(0, 0), Size::new(2, 2));
        l.fill(Rect::new(1, 1, 99, 99), LayerCell::Shade);
        assert_eq!(l.get(1, 1), LayerCell::Shade);
        assert_eq!(l.get(0, 0), LayerCell::Transparent);
    }

    #[test]
    fn local_bounds_is_always_origin_zero_regardless_of_layer_origin() {
        let l = Layer::new(Point::new(70, 20), Size::new(4, 5));
        assert_eq!(l.local_bounds(), Rect::new(0, 0, 4, 5));
        // `bounds()` reflects the real screen-space origin.
        assert_eq!(l.bounds(), Rect::new(70, 20, 4, 5));
    }

    #[test]
    fn stack_get_and_iter_observe_bottom_to_top_order() {
        let mut stack = LayerStack::new();
        stack.push(Layer::new(Point::new(0, 0), Size::new(1, 1)));
        stack.push(Layer::new(Point::new(5, 5), Size::new(1, 1)));
        stack.push(Layer::new(Point::new(9, 9), Size::new(1, 1)));

        assert_eq!(stack.get(0).expect("layer").origin, Point::new(0, 0));
        assert_eq!(stack.get(1).expect("layer").origin, Point::new(5, 5));
        assert_eq!(stack.get(2).expect("layer").origin, Point::new(9, 9));
        assert!(stack.get(3).is_none());

        let origins: Vec<Point> = stack.iter().map(|l| l.origin).collect();
        assert_eq!(
            origins,
            vec![Point::new(0, 0), Point::new(5, 5), Point::new(9, 9)]
        );
    }

    #[test]
    fn shade_darkens_a_lower_layers_opaque_cell_not_just_the_base() {
        let base = base_filled(3, 1, b'.');
        let mut out = CellBuffer::new(3, 1);
        let mut stack = LayerStack::new();

        // Lower layer paints a bright character.
        stack.push(Layer::filled(
            Point::new(0, 0),
            Size::new(2, 1),
            Cell::new(b'P', 0xF, 0x0, false),
        ));
        // Upper layer casts a shadow across one of its cells.
        let mut shadow = Layer::new(Point::new(1, 0), Size::new(2, 1));
        shadow.set(0, 0, LayerCell::Shade);
        shadow.set(1, 0, LayerCell::Shade);
        stack.push(shadow);

        stack.composite(&base, &mut out);

        // Shaded cell over the lower layer keeps that layer's character.
        assert_eq!(out.get(1, 0).ch, b'P');
        assert_eq!(out.get(1, 0).attr, SHADE_ATTR);
        // Shaded cell over the base keeps the base character.
        assert_eq!(out.get(2, 0).ch, b'.');
        assert_eq!(out.get(2, 0).attr, SHADE_ATTR);
        // Unshaded part of the lower layer is untouched.
        assert_eq!(out.get(0, 0).ch, b'P');
        assert_eq!(out.get(0, 0).attr, Cell::new(b'P', 0xF, 0x0, false).attr);
    }
}
