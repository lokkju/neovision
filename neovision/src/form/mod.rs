//! Modal form widgets.

pub mod model;
pub mod render;

pub use model::{
    ButtonRole, ChoiceOption, Field, FieldKind, FormEvent, FormOutcome, FormState, Popup,
};
pub use render::{render, render_themed, render_with_cursor, HotkeyAttrs, Theme};
#[allow(deprecated)]
pub use render::{FormTheme, HotkeyTheme};
