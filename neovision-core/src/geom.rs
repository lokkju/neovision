//! Integer geometry for cell grids.
//!
//! Coordinates are unsigned: layers never sit at negative origins, because
//! CUA panels reposition to fit rather than hanging off an edge. Clipping at
//! the right and bottom edges is still performed — that is what keeps an
//! oversized panel from panicking.

/// A position on a cell grid.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Point {
    pub x: u16,
    pub y: u16,
}

impl Point {
    pub const fn new(x: u16, y: u16) -> Self {
        Self { x, y }
    }
}

/// Dimensions of a cell region.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Size {
    pub w: u16,
    pub h: u16,
}

impl Size {
    pub const fn new(w: u16, h: u16) -> Self {
        Self { w, h }
    }

    /// True when the region covers no cells at all.
    pub const fn is_empty(self) -> bool {
        self.w == 0 || self.h == 0
    }
}

/// A rectangular cell region. `right()` and `bottom()` are exclusive.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Rect {
    pub origin: Point,
    pub size: Size,
}

impl Rect {
    pub const fn new(x: u16, y: u16, w: u16, h: u16) -> Self {
        Self {
            origin: Point::new(x, y),
            size: Size::new(w, h),
        }
    }

    pub const fn left(&self) -> u16 {
        self.origin.x
    }

    pub const fn top(&self) -> u16 {
        self.origin.y
    }

    /// Exclusive right edge. Saturates rather than overflowing.
    pub const fn right(&self) -> u16 {
        self.origin.x.saturating_add(self.size.w)
    }

    /// Exclusive bottom edge. Saturates rather than overflowing.
    pub const fn bottom(&self) -> u16 {
        self.origin.y.saturating_add(self.size.h)
    }

    /// True when `p` falls inside this rect.
    pub fn contains(&self, p: Point) -> bool {
        p.x >= self.left() && p.x < self.right() && p.y >= self.top() && p.y < self.bottom()
    }

    /// The region common to both rects, or `None` when they are disjoint or
    /// either is zero-area.
    pub fn intersect(&self, other: &Rect) -> Option<Rect> {
        let x0 = self.left().max(other.left());
        let y0 = self.top().max(other.top());
        let x1 = self.right().min(other.right());
        let y1 = self.bottom().min(other.bottom());
        if x1 <= x0 || y1 <= y0 {
            return None;
        }
        Some(Rect::new(x0, y0, x1 - x0, y1 - y0))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rect_edges_are_exclusive_on_right_and_bottom() {
        let r = Rect::new(2, 3, 4, 5);
        assert_eq!(r.left(), 2);
        assert_eq!(r.top(), 3);
        assert_eq!(r.right(), 6);
        assert_eq!(r.bottom(), 8);
    }

    #[test]
    fn rect_contains_respects_exclusive_edges() {
        let r = Rect::new(1, 1, 2, 2);
        assert!(r.contains(Point::new(1, 1)));
        assert!(r.contains(Point::new(2, 2)));
        assert!(!r.contains(Point::new(3, 2)));
        assert!(!r.contains(Point::new(2, 3)));
        assert!(!r.contains(Point::new(0, 1)));
    }

    #[test]
    fn intersect_overlapping_returns_common_area() {
        let a = Rect::new(0, 0, 4, 4);
        let b = Rect::new(2, 2, 4, 4);
        assert_eq!(a.intersect(&b), Some(Rect::new(2, 2, 2, 2)));
    }

    #[test]
    fn intersect_disjoint_returns_none() {
        let a = Rect::new(0, 0, 2, 2);
        let b = Rect::new(5, 5, 2, 2);
        assert_eq!(a.intersect(&b), None);
    }

    #[test]
    fn intersect_touching_edges_returns_none() {
        // a's exclusive right edge is 2, b starts at 2 — they share no cell.
        let a = Rect::new(0, 0, 2, 2);
        let b = Rect::new(2, 0, 2, 2);
        assert_eq!(a.intersect(&b), None);
    }

    #[test]
    fn intersect_containment_returns_inner() {
        let outer = Rect::new(0, 0, 10, 10);
        let inner = Rect::new(3, 4, 2, 2);
        assert_eq!(outer.intersect(&inner), Some(inner));
        assert_eq!(inner.intersect(&outer), Some(inner));
    }

    #[test]
    fn intersect_zero_size_returns_none() {
        let a = Rect::new(0, 0, 4, 4);
        let zero = Rect::new(1, 1, 0, 3);
        assert_eq!(a.intersect(&zero), None);
    }

    #[test]
    fn size_is_empty_when_either_dimension_is_zero() {
        assert!(Size::new(0, 5).is_empty());
        assert!(Size::new(5, 0).is_empty());
        assert!(!Size::new(1, 1).is_empty());
    }

    #[test]
    fn rect_edges_saturate_instead_of_overflowing() {
        let r = Rect::new(u16::MAX - 1, u16::MAX - 1, 10, 10);
        assert_eq!(r.right(), u16::MAX);
        assert_eq!(r.bottom(), u16::MAX);
        // A saturated rect still intersects the grid it overhangs.
        let screen = Rect::new(0, 0, u16::MAX, u16::MAX);
        assert_eq!(
            r.intersect(&screen),
            Some(Rect::new(u16::MAX - 1, u16::MAX - 1, 1, 1))
        );
    }
}
