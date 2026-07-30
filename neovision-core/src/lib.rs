//! The character-cell substrate underneath [neovision].
//!
//! A [`CellBuffer`] is a grid of [`Cell`]s in VGA text-mode byte layout: one
//! CP437 character byte plus one attribute byte, so [`CellBuffer::as_bytes`]
//! is directly comparable to a memory dump from a real text-mode display.
//! [`Layer`]s stack over that grid and [`LayerStack::composite`] flattens
//! them — the model Turbo Vision used for overlapping windows and the shadows
//! its dialogs cast.
//!
//! Nothing here knows about terminals, framebuffers, or event loops, and the
//! crate has zero dependencies. It is the common denominator any text-mode UI
//! can build on; neovision's widgets are one such consumer.
//!
//! [neovision]: https://docs.rs/neovision
#![no_std]

extern crate alloc;

pub mod cell;
pub mod cp437;
pub mod cursor;
pub mod draw;
pub mod geom;
pub mod layer;

pub use cell::{Cell, CellBuffer};
pub use cursor::{CursorShape, TextCursor};
pub use draw::{BoxChars, CellCanvas, CellDraw};
pub use geom::{Point, Rect, Size};
pub use layer::{Layer, LayerCell, LayerStack, SHADE_ATTR};
