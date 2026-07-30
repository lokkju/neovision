//! A CUA / Turbo-Vision-style widget toolkit for character-cell text UIs.
//!
//! neovision draws DOS-era modal forms — framed dialogs, entry fields, choice
//! pickers, toggles, buttons — onto a character-cell grid, and hands the result
//! back as [`Layer`]s plus an optional [`TextCursor`]. It never touches a
//! terminal, a framebuffer, or an event loop: rendering is a pure function from
//! state to cells, so the same form renders identically to a VGA text buffer, a
//! pixel framebuffer, a wasm canvas, or a terminal. The host decides how a cell
//! becomes a pixel.
//!
//! # Layout
//!
//! [`FormState`] holds the fields and focus; [`render()`] turns it into layers.
//! Input arrives as [`FormEvent`] values — the toolkit's own key abstraction,
//! not any host's — and [`FormState::handle`] returns a [`FormOutcome`]
//! describing what the form did.
//!
//! The model is generic over the caller's action type `A`. It stores and
//! returns `A` values but never inspects them, so the toolkit carries no
//! dependency on any consuming application.
//!
//! # Substrate
//!
//! The cell primitives live in [`neovision_core`] and are re-exported here, so
//! `cargo add neovision` is enough to get both the widgets and the substrate
//! they draw on.
//!
//! ```
//! use neovision::{Cell, CellBuffer, LayerStack};
//!
//! let mut base = CellBuffer::new(80, 25);
//! base.fill(Cell::new(b' ', 0x7, 0x1, false));
//!
//! let stack = LayerStack::new();
//! let mut screen = CellBuffer::new(80, 25);
//! stack.composite(&base, &mut screen);
//!
//! assert_eq!(screen.get(0, 0).bg(), 0x1);
//! ```
#![no_std]

extern crate alloc;

pub mod form;

pub use form::{
    render, render_themed, render_with_cursor, ButtonRole, ChoiceOption, Field, FieldKind,
    FormEvent, FormOutcome, FormState, HotkeyAttrs, Popup, Theme,
};

/// Former names, kept so a 0.1.0 dependant gets a warning rather than a break.
#[allow(deprecated)]
pub use form::{FormTheme, HotkeyTheme};

/// The character-cell substrate neovision renders onto.
pub use neovision_core;

pub use neovision_core::{
    BoxChars, Cell, CellBuffer, CellCanvas, CellDraw, CursorShape, Layer, LayerCell, LayerStack,
    Point, Rect, Size, TextCursor, SHADE_ATTR,
};
