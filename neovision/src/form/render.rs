//! Draw a [`FormState`] into cell layers.
//!
//! Pure: takes state, returns layers, touches nothing. The caller pushes the
//! result onto its layer stack. Layers come back bottom-to-top, so pushing
//! them in order gives the right z-order.

use alloc::string::String;
use alloc::vec::Vec;

use super::model::{parse_mnemonic, Field, FieldKind, FormState, Popup};
// Only the test module's `use super::*` needs these; the renderer itself
// never constructs a `ChoiceOption` or feeds a `FormEvent`.
#[cfg(test)]
use super::model::{ChoiceOption, FormEvent};
use neovision_core::{
    BoxChars, Cell, CellCanvas, CellDraw, CursorShape, Layer, LayerCell, Point, Rect, Size,
    TextCursor,
};

/// Attributes for the accelerator letter in a label.
///
/// Two of them, because a hotkey has to stay legible on both backgrounds a
/// label is drawn on, and one attribute cannot. Turbo Vision's palettes made
/// the same split, carrying separate "shortcut" entries for normal and
/// selected text.
#[derive(Debug, Clone, Copy)]
pub struct HotkeyAttrs {
    /// On an unfocused row, over `normal`.
    pub normal: u8,
    /// On the focused row, over `selected`.
    pub selected: u8,
}

/// VGA attributes for the form's parts.
///
/// Kept as a plain struct of `&'static` attribute bytes (rather than baked
/// into the drawing code) because a themed pixel renderer is planned: it will
/// want to vary these per skin without touching `render()` itself.
#[derive(Debug, Clone, Copy)]
pub struct Theme {
    /// Frame and labels.
    pub normal: u8,
    /// The bracketed value text of an editable field — distinct from
    /// `normal` so data visually stands out from its label, CUA-style.
    pub value: u8,
    /// The focused row and the highlighted popup option.
    pub selected: u8,
    /// Read-only rows (label and value both).
    pub dim: u8,
    /// The panel title.
    pub title: u8,
    /// Caret colour, for a themed host that draws the caret itself.
    ///
    /// [`render`] never reads this. The caret is reported out-of-band as a
    /// [`TextCursor`] by [`render_with_cursor`] — carrying position and shape,
    /// but deliberately not colour — so that drawing it cannot overwrite the
    /// glyph underneath.
    ///
    /// The **foreground nibble is the caret colour**; the background nibble is
    /// unused, since a caret is drawn over a cell that already has one. It must
    /// contrast against [`selected`](Self::selected), not against
    /// [`normal`](Self::normal): a caret only ever appears on the focused row,
    /// which is painted in the selection attribute.
    ///
    /// A host is free to ignore this and derive the colour from the cell
    /// instead — which is what [`CursorShape::Overtype`] prescribes with its
    /// reverse video, and what VGA hardware does, drawing the cursor in the
    /// character's own foreground colour.
    pub cursor: u8,
    /// How to mark the accelerator letter in a label, or `None` to leave
    /// labels unmarked.
    ///
    /// `None` is for hosts that cannot deliver
    /// [`FormEvent::Hotkey`](crate::FormEvent::Hotkey) at all —
    /// an embedded keypad, a canvas that swallows modifiers. Such a host must
    /// clear this, or the form underlines letters promising an affordance
    /// nothing will honour. The `~X~` markers are stripped from labels either
    /// way, so the text reads correctly regardless.
    pub hotkey: Option<HotkeyAttrs>,
    /// The whole-value selection highlight of a selected Number field. Reverse
    /// of the focused row bar (0x71) so selected digits read as a highlight
    /// block on the inverse row.
    pub selection: u8,
}

impl Theme {
    /// CUA convention: light grey on blue, inverted for the selection bar,
    /// bright white values and a yellow title accent.
    pub const DEFAULT: Theme = Theme {
        normal: 0x17,   // light grey on blue
        value: 0x1F,    // bright white on blue
        selected: 0x71, // blue on light grey
        dim: 0x18,      // dark grey on blue
        title: 0x1E,    // bright yellow on blue
        // Black caret. It has to contrast against `selected`'s light-grey bar,
        // which is the only background a caret is ever drawn on — a bright
        // caret reads well against the blue panel and then vanishes on the
        // focused row, which is the one place it actually appears.
        cursor: 0x70,
        hotkey: Some(HotkeyAttrs {
            normal: 0x1E,   // bright yellow on blue, the CUA accelerator colour
            selected: 0x74, // red on light grey: the same accent, legible on the bar
        }),
        selection: 0x17, // grey-on-blue: reverse of the 0x71 focused row bar
    };
}

/// Visible width of a [`FieldKind::Text`] value, inside its brackets.
const TEXT_VISIBLE_W: u16 = VALUE_W - 2;

/// First visible character of a text field, derived from the caret alone.
///
/// Stateless on purpose: keeping a `first_visible` in the model would be a
/// second source of truth that has to be nudged back into agreement with the
/// caret on every edit. Deriving it means the two cannot disagree.
fn text_scroll(cursor: usize) -> usize {
    cursor.saturating_sub(TEXT_VISIBLE_W as usize - 1)
}

/// Former name of [`Theme`].
///
/// Renamed because it themes more than a form: the panel, its labels and
/// values, the selection bar, the caret and now accelerators, with clusters
/// and scrollbars still to come.
#[deprecated(since = "0.2.0", note = "renamed to `Theme`")]
pub type FormTheme = Theme;

/// Former name of [`HotkeyAttrs`].
#[deprecated(since = "0.2.0", note = "renamed to `HotkeyAttrs`")]
pub type HotkeyTheme = HotkeyAttrs;

/// Width of the label column, in cells.
const LABEL_W: u16 = 14;
/// Width of the value column, in cells.
const VALUE_W: u16 = 22;
/// Inner width: label + gap + value.
const INNER_W: u16 = LABEL_W + 1 + VALUE_W;

/// Minimum inner width (excluding the brackets) of a button's chrome, so a
/// short label like "OK" still reads as a button rather than a bare word in
/// brackets. Longer labels ("Cancel") just get their usual 1-cell padding.
const BUTTON_MIN_INNER: u16 = 6;
/// Gap between adjacent buttons on a shared row.
const BUTTON_GAP: u16 = 4;

/// Write a label, stripping `~X~` markers and marking the accelerator.
///
/// Returns nothing useful — it is a draw, not a measurement — but note that
/// the marker characters never occupy a cell, so a label's rendered width is
/// its text width whether or not it claims an accelerator.
fn write_label<C: CellDraw + ?Sized>(
    canvas: &mut C,
    at: Point,
    label: &str,
    attr: u8,
    hotkey_attr: Option<u8>,
) {
    let (text, mnemonic) = parse_mnemonic(label);
    canvas.write_str(at, &text, attr);
    if let (Some((_, idx)), Some(hot)) = (mnemonic, hotkey_attr) {
        // Repaint just the one cell, so the accelerator picks up its own
        // attribute without the label being drawn twice.
        let x = at.x.saturating_add(idx as u16);
        if let Some(ch) = text.chars().nth(idx) {
            canvas.put(
                x,
                at.y,
                Cell {
                    ch: cp437_byte(ch),
                    attr: hot,
                },
            );
        }
    }
}

/// One `char` as the CP437 byte a cell holds.
fn cp437_byte(ch: char) -> u8 {
    neovision_core::cp437::from_char(ch).unwrap_or(b'?')
}

/// Truncate-or-pad `s` to exactly `width` chars.
///
/// Operates on `chars()`, not bytes: `value_text` can return the em dash
/// (`\u{2014}`) for an unset `Choice`, and `write_str` renders any non-ASCII
/// char as a single `?` cell — one char in, one cell out, regardless of its
/// UTF-8 byte length. Byte-slicing here would either panic on a multi-byte
/// boundary or miscount the rendered width.
fn fit(s: &str, width: usize) -> String {
    let mut out: String = s.chars().take(width).collect();
    let n = out.chars().count();
    if n < width {
        // `repeat().take()` rather than `repeat_n`, which would raise the MSRV
        // to 1.82 for no gain.
        out.extend(core::iter::repeat(' ').take(width - n));
    }
    out
}

/// The value column for a non-Button field: bracketed and padded to
/// `VALUE_W` for every editable kind (`[ value... ]`), so the closing
/// bracket lines up in a column down the panel; unbracketed (but still
/// padded/truncated to `VALUE_W`) for `ReadOnly`, whose absence of chrome is
/// itself part of what marks it non-editable — a dim attribute alone is not
/// a strong enough signal that a row cannot be edited.
///
/// Truncating here — rather than relying on `write_str`'s canvas-edge
/// truncation — is what keeps an oversized value from overwriting the right
/// border: `write_str` only stops at the layer's own edge, which for the
/// body layer includes the border column itself.
fn value_column<A>(field: &Field<A>, focused: bool) -> String {
    match &field.kind {
        FieldKind::ReadOnly(_) => fit(&value_text(field), VALUE_W as usize),
        FieldKind::Number {
            value,
            buffer,
            unit,
            min,
            max,
            ..
        } => {
            // A focused, emptied buffer (select -> Backspace) must render as
            // genuinely empty — `[ ]`, cursor at column 0 — not fall back to
            // the stale committed `value`. Showing `[120 ]` with the edit
            // cursor drawn on top of it would contradict what the user just
            // did. The fallback to `value` is only correct when unfocused,
            // where there is no live edit in progress to contradict.
            let digits = if buffer.is_empty() && !focused {
                alloc::format!("{value}")
            } else {
                buffer.clone()
            };
            // Fixed-width digit slot (4-cap) + trailing cursor slot, so `]`,
            // the unit and the range never shift as digits are typed.
            let s = alloc::format!("[{:<4} ]{unit} ({min}-{max})", digits);
            fit(&s, VALUE_W as usize)
        }
        FieldKind::Text { buffer, cursor, .. } => {
            let first = text_scroll(*cursor);
            let window: String = buffer
                .chars()
                .skip(first)
                .take(TEXT_VISIBLE_W as usize)
                .collect();
            alloc::format!("[{}]", fit(&window, TEXT_VISIBLE_W as usize))
        }
        _ => {
            let inner_w = VALUE_W.saturating_sub(2) as usize;
            alloc::format!("[{}]", fit(&value_text(field), inner_w))
        }
    }
}

/// Render one button's chrome, e.g. `[  OK  ]` or `[ Cancel ]`.
///
/// Padding is 1 cell each side by default, widened to meet
/// [`BUTTON_MIN_INNER`] for short labels — that's what makes a 2-char label
/// like "OK" read as a button rather than a bracketed word.
fn button_chrome_with(label: &str, is_default: bool) -> String {
    let mut s = button_chrome(label);
    if is_default {
        // CP437 guillemets mark the button Enter presses. Same width as the
        // square brackets they replace, so nothing shifts.
        let mut chars: Vec<char> = s.chars().collect();
        if let Some(first) = chars.first_mut() {
            *first = '\u{00AB}';
        }
        if let Some(last) = chars.last_mut() {
            *last = '\u{00BB}';
        }
        s = chars.into_iter().collect();
    }
    s
}

fn button_chrome(label: &str) -> String {
    let len = label.chars().count() as u16;
    let inner_w = (len + 2).max(BUTTON_MIN_INNER);
    let pad = inner_w - len;
    let left = pad / 2;
    let right = pad - left;
    let mut s = String::new();
    s.push('[');
    s.extend(core::iter::repeat(' ').take(left as usize));
    s.push_str(label);
    s.extend(core::iter::repeat(' ').take(right as usize));
    s.push(']');
    s
}

fn value_text<A>(field: &Field<A>) -> alloc::string::String {
    use alloc::string::ToString;
    match &field.kind {
        FieldKind::Choice { options, selected } => selected
            .and_then(|i| options.get(i))
            .map(|o| o.label.clone())
            .unwrap_or_else(|| "\u{2014}".to_string()),
        FieldKind::Number {
            value,
            buffer,
            unit,
            ..
        } => {
            if buffer.is_empty() {
                alloc::format!("{value}{unit}")
            } else {
                alloc::format!("{buffer}{unit}")
            }
        }
        FieldKind::Text { buffer, .. } => buffer.clone(),
        FieldKind::Toggle { on, .. } => if *on { "Yes" } else { "No" }.to_string(),
        FieldKind::ReadOnly(s) => s.clone(),
        FieldKind::Button { label, .. } => parse_mnemonic(label).0,
    }
}

/// A solid `Shade` layer covering `r`, offset one cell right and down.
///
/// Deliberately solid, not L-shaped: the panel is pushed *after* this and
/// overwrites the overlapping cells opaquely, so the visible result is the
/// usual CUA L. Relying on z-order keeps this function trivial — carving the
/// L here would mean duplicating the panel's geometry.
fn shadow_layer(r: Rect, screen: Size) -> Layer {
    // Clamp the offset origin itself, not just the resulting width/height:
    // on a screen too small to hold `r` at all (e.g. h == 0), `r.top() + 1`
    // can already exceed `screen.h`. Clamping only `w`/`h` from an
    // unclamped origin still yields a technically-empty layer positioned
    // past the screen edge, which fails the "every rect is clamped to the
    // screen" contract even though nothing visible is drawn.
    let x = (r.left() + 1).min(screen.w);
    let y = (r.top() + 1).min(screen.h);
    let w = r.size.w.min(screen.w.saturating_sub(x));
    let h = r.size.h.min(screen.h.saturating_sub(y));
    let mut layer = Layer::new(Point::new(x, y), Size::new(w, h));
    layer.fill(layer.local_bounds(), LayerCell::Shade);
    layer
}

/// Centre a rect of `size` on `screen`, clamped so it never goes off the
/// left or top edge (neovision-core coordinates are unsigned).
fn centred(size: Size, screen: Size) -> Rect {
    let w = size.w.min(screen.w);
    let h = size.h.min(screen.h);
    let x = (screen.w.saturating_sub(w)) / 2;
    let y = (screen.h.saturating_sub(h)) / 2;
    Rect::new(x, y, w, h)
}

/// Index of the first field in the trailing run of `FieldKind::Button`
/// fields, or `fields.len()` if it has none.
///
/// Only a *trailing* run collapses onto a shared row — the model and focus
/// traversal are untouched, so a hypothetical button in the middle of the
/// field list (which nothing currently builds) would still get its own row
/// rather than being silently merged into a distant button row.
fn button_run_start<A>(fields: &[Field<A>]) -> usize {
    let trailing = fields
        .iter()
        .rev()
        .take_while(|f| matches!(f.kind, FieldKind::Button { .. }))
        .count();
    fields.len() - trailing
}

/// Draw `state` into layers, bottom-to-top, plus the text-cursor descriptor
/// (if any) for the focused field. Shared by `render` and `render_with_cursor`
/// so the layout math (panel origin, per-row field position) is computed
/// exactly once rather than duplicated between a layers-only and a
/// layers-plus-cursor entry point.
fn render_impl<A>(
    state: &FormState<A>,
    screen: Size,
    theme: Theme,
) -> (Vec<Layer>, Option<TextCursor>) {
    let mut layers = Vec::new();
    let mut text_cursor: Option<TextCursor> = None;

    let fields = state.fields();
    let button_start = button_run_start(fields);
    let has_buttons = button_start < fields.len();
    let field_rows = button_start as u16;
    // A collapsed button row also gets a separator rule above it, so the two
    // extra rows (separator + button row) replace what would otherwise be
    // one row per button.
    let trailer_rows = if has_buttons { 2 } else { 0 };
    let panel_h = field_rows + trailer_rows + 2;
    let panel = centred(Size::new(INNER_W + 2, panel_h), screen);

    layers.push(shadow_layer(panel, screen));

    let mut body = Layer::new(panel.origin, panel.size);
    body.fill(
        body.local_bounds(),
        LayerCell::Opaque(Cell {
            ch: b' ',
            attr: theme.normal,
        }),
    );
    body.draw_box(body.local_bounds(), BoxChars::DOUBLE, theme.normal);

    // Title centred in the top edge, padded so the frame does not touch it.
    let title = alloc::format!(" {} ", state.title);
    let tx = (panel.size.w.saturating_sub(title.len() as u16)) / 2;
    body.write_str(Point::new(tx, 0), &title, theme.title);

    for (i, field) in fields[..button_start].iter().enumerate() {
        let row = i as u16 + 1;
        let focused = i == state.focus();
        let read_only = matches!(field.kind, FieldKind::ReadOnly(_));
        let label_attr = if focused {
            theme.selected
        } else if read_only {
            theme.dim
        } else {
            theme.normal
        };
        // The value gets its own attribute: normal editable rows show it in
        // `theme.value` so data stands out from its label, but a
        // focused or read-only row stays a single solid colour across the
        // whole row, matching `label_attr`.
        let value_attr = if focused || read_only {
            label_attr
        } else {
            theme.value
        };
        // Paint the whole row so the selection bar spans the panel.
        body.fill(
            Rect::new(1, row, INNER_W, 1),
            LayerCell::Opaque(Cell {
                ch: b' ',
                attr: label_attr,
            }),
        );
        write_label(
            &mut body,
            Point::new(2, row),
            field.label,
            label_attr,
            theme
                .hotkey
                .map(|h| if focused { h.selected } else { h.normal }),
        );
        body.write_str(
            Point::new(2 + LABEL_W, row),
            &value_column(field, focused),
            value_attr,
        );
        // Selection highlight for a focused, selected Number field.
        // `selected` (whole-value selection on entry, Turbo Vision-style)
        // repaints every digit cell in `theme.selection`. The edit caret
        // itself is deliberately not drawn into the cell buffer — see
        // `text_cursor` below, which derives a `TextCursor` descriptor
        // instead, leaving the host to draw the caret so it never overwrites
        // the digit glyph beneath it.
        if focused {
            if let FieldKind::Number {
                buffer, selected, ..
            } = &field.kind
            {
                if *selected {
                    let digits_start = 2 + LABEL_W + 1; // past the '['
                                                        // `Layer::get` returns a `LayerCell` enum, not a `Cell`,
                                                        // so the digits already drawn by `value_column` above
                                                        // can't be read back and re-attributed in place — instead
                                                        // re-write each digit char from `buffer` with the
                                                        // selection attribute.
                    for (i, ch) in buffer.bytes().enumerate() {
                        body.put(
                            digits_start + i as u16,
                            row,
                            Cell {
                                ch,
                                attr: theme.selection,
                            },
                        );
                    }
                }
            }
        }

        // Text cursor: only for a focused, actively-edited (not selected)
        // Number field. Screen cell = panel origin + local field position.
        if focused {
            let entry = match &field.kind {
                FieldKind::Number {
                    cursor,
                    selected,
                    overtype,
                    ..
                } => Some((*cursor, *selected, *overtype)),
                // A text field scrolls, so the caret's column is its offset
                // within the visible window rather than within the buffer.
                FieldKind::Text {
                    cursor,
                    selected,
                    overtype,
                    ..
                } => Some((*cursor - text_scroll(*cursor), *selected, *overtype)),
                _ => None,
            };
            if let Some((cursor, selected, overtype)) = entry {
                if !selected {
                    let local_x = 2 + LABEL_W + 1 + cursor as u16; // past label, gap, '['
                    text_cursor = Some(TextCursor {
                        col: panel.origin.x + local_x,
                        row: panel.origin.y + row,
                        shape: if overtype {
                            CursorShape::Overtype
                        } else {
                            CursorShape::Insert
                        },
                    });
                }
            }
        }
    }

    if has_buttons {
        let sep_row = field_rows + 1;
        let btn_row = sep_row + 1;

        // Separator: tee glyphs where the rule meets the frame, matching the
        // border's own DOUBLE box style.
        body.put(
            0,
            sep_row,
            Cell {
                ch: BoxChars::DOUBLE.tee_l,
                attr: theme.normal,
            },
        );
        body.draw_hline(
            Point::new(1, sep_row),
            INNER_W,
            BoxChars::DOUBLE.h,
            theme.normal,
        );
        body.put(
            INNER_W + 1,
            sep_row,
            Cell {
                ch: BoxChars::DOUBLE.tee_r,
                attr: theme.normal,
            },
        );

        // Button row background, plain — only the focused button's own
        // chrome (not the row) gets the selected attribute.
        body.fill(
            Rect::new(1, btn_row, INNER_W, 1),
            LayerCell::Opaque(Cell {
                ch: b' ',
                attr: theme.normal,
            }),
        );

        let chromes: Vec<(usize, String)> = fields[button_start..]
            .iter()
            .enumerate()
            .map(|(k, f)| {
                let is_default = matches!(f.kind, FieldKind::Button { default: true, .. });
                (
                    button_start + k,
                    button_chrome_with(&value_text(f), is_default),
                )
            })
            .collect();
        let gap = BUTTON_GAP;
        let total_w: u16 = chromes
            .iter()
            .map(|(_, s)| s.chars().count() as u16)
            .sum::<u16>()
            + gap.saturating_mul(chromes.len().saturating_sub(1) as u16);
        let mut x = 1 + (INNER_W.saturating_sub(total_w)) / 2;
        for (idx, chrome) in &chromes {
            // The focused button's *whole* chrome (brackets and padding, not
            // just the label) carries the selected attribute. Highlighting the
            // label alone reads as broken chrome rather than as focus.
            let attr = if *idx == state.focus() {
                theme.selected
            } else {
                theme.normal
            };
            body.write_str(Point::new(x, btn_row), chrome, attr);

            // A button's accelerator is marked in its own label. Its column
            // inside the chrome is the opening bracket plus the left padding
            // plus its index in the label — computed rather than searched, so
            // a label whose accelerator letter also appears earlier in it
            // still marks the right cell.
            if let (Some(hot), Some(FieldKind::Button { label, .. })) =
                (theme.hotkey, fields.get(*idx).map(|f| &f.kind))
            {
                let (text, mnemonic) = parse_mnemonic(label);
                if let Some((wanted, idx_in_label)) = mnemonic {
                    let len = text.chars().count() as u16;
                    let inner_w = (len + 2).max(BUTTON_MIN_INNER);
                    let offset = 1 + (inner_w - len) / 2 + idx_in_label as u16;
                    let hot_attr = if *idx == state.focus() {
                        hot.selected
                    } else {
                        hot.normal
                    };
                    body.put(
                        x.saturating_add(offset),
                        btn_row,
                        Cell {
                            ch: cp437_byte(wanted),
                            attr: hot_attr,
                        },
                    );
                }
            }

            x = x.saturating_add(chrome.chars().count() as u16 + gap);
        }
    }

    layers.push(body);

    if let Some(popup) = state.popup() {
        push_popup(&mut layers, state, popup, panel, screen, theme);
    }

    (layers, text_cursor)
}

/// Draw `state` into layers, bottom-to-top.
pub fn render<A>(state: &FormState<A>, screen: Size) -> Vec<Layer> {
    render_impl(state, screen, Theme::DEFAULT).0
}

/// As `render`, but also returns the text-cursor descriptor for the
/// focused, actively-edited field (`None` if no field is being edited, e.g.
/// a Choice/Toggle/Button is focused, or the focused Number field's value is
/// whole-selected rather than mid-edit). The cursor is never written into
/// the cell buffer, so a host renders it separately (pixel scanlines, or a
/// text-mode BIOS cursor) without risking overwriting the digit glyph.
pub fn render_with_cursor<A>(
    state: &FormState<A>,
    screen: Size,
) -> (Vec<Layer>, Option<TextCursor>) {
    render_impl(state, screen, Theme::DEFAULT)
}

/// As [`render_with_cursor`], but drawn with the caller's own [`Theme`].
///
/// This is what makes `Theme` more than documentation: a host with its own
/// skin varies the attributes here rather than reaching into the renderer. It
/// is also the only way to clear [`Theme::hotkey`], which a host that
/// cannot deliver `FormEvent::Hotkey` should do, so that labels stop
/// advertising accelerators nothing will honour.
pub fn render_themed<A>(
    state: &FormState<A>,
    screen: Size,
    theme: Theme,
) -> (Vec<Layer>, Option<TextCursor>) {
    render_impl(state, screen, theme)
}

fn push_popup<A>(
    layers: &mut Vec<Layer>,
    state: &FormState<A>,
    popup: &Popup,
    panel: Rect,
    screen: Size,
    theme: Theme,
) {
    let FieldKind::Choice { options, .. } = &state.fields()[popup.field].kind else {
        return;
    };
    if options.is_empty() {
        return;
    }

    let widest = options.iter().map(|o| o.label.len()).max().unwrap_or(0) as u16;
    let inner_w = widest.max(4).min(screen.w.saturating_sub(4));
    let w = inner_w + 2;

    // Sit under the field, then clamp so the popup stays on screen.
    let y = (panel.top() + popup.field as u16 + 2).min(screen.h.saturating_sub(3));
    let max_rows = screen.h.saturating_sub(y).saturating_sub(1);
    // A framed list needs at least three rows: top border, one option, bottom
    // border. On a screen too short for that, draw no popup at all rather than
    // returning a layer that overhangs.
    if max_rows < 3 {
        return;
    }
    let visible = options.len().min((max_rows - 2) as usize);
    let h = visible as u16 + 2;
    let x = (panel.left() + LABEL_W).min(screen.w.saturating_sub(w));

    // Scroll so the highlight is always visible.
    let first = popup.highlight.saturating_sub(visible.saturating_sub(1));

    let rect = Rect::new(x, y, w, h);
    layers.push(shadow_layer(rect, screen));

    let mut list = Layer::new(rect.origin, rect.size);
    list.fill(
        list.local_bounds(),
        LayerCell::Opaque(Cell {
            ch: b' ',
            attr: theme.normal,
        }),
    );
    list.draw_box(list.local_bounds(), BoxChars::SINGLE, theme.normal);

    for (row, idx) in (first..first + visible).enumerate() {
        let Some(opt) = options.get(idx) else { break };
        let attr = if idx == popup.highlight {
            theme.selected
        } else {
            theme.normal
        };
        let y = row as u16 + 1;
        list.fill(
            Rect::new(1, y, inner_w, 1),
            LayerCell::Opaque(Cell { ch: b' ', attr }),
        );
        list.write_str(Point::new(1, y), &opt.label, attr);
    }
    layers.push(list);
}

#[cfg(test)]
mod tests {
    use super::super::model::ButtonRole;
    use super::*;
    use alloc::string::ToString;
    use neovision_core::{CellBuffer, LayerStack};

    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    enum TestOp {
        A,
        B,
    }

    /// OK and Cancel both present (not just OK) so the layout tests exercise
    /// the actual side-by-side button row, not a degenerate one-button case.
    fn marked_form() -> FormState<TestOp> {
        FormState::new(
            "T",
            alloc::vec![
                Field {
                    label: "~S~ound",
                    kind: FieldKind::Toggle {
                        on: true,
                        on_action: TestOp::A,
                        off_action: TestOp::B,
                    },
                    restore: alloc::vec![],
                },
                Field {
                    label: "",
                    kind: FieldKind::Button {
                        label: "~O~K",
                        role: ButtonRole::Accept,
                        action: None,
                        default: true
                    },
                    restore: alloc::vec![],
                },
            ],
        )
    }

    #[test]
    fn the_former_theme_names_still_resolve() {
        // A 0.1.0 dependant should get a deprecation warning, not a break.
        #[allow(deprecated)]
        let _t: FormTheme = Theme::DEFAULT;
        #[allow(deprecated)]
        let _h: Option<HotkeyTheme> = Theme::DEFAULT.hotkey;
    }

    #[test]
    fn a_marked_label_renders_without_its_tildes() {
        let screen = Size::new(80, 25);
        let buf = flatten(render(&marked_form(), screen), screen);
        let row = find_row(&buf, "Sound").expect("the label reads as Sound");
        assert!(
            !row_text(&buf, row).contains('~'),
            "markers must never occupy a cell"
        );
    }

    #[test]
    fn the_accelerator_letter_carries_its_own_attribute() {
        let screen = Size::new(80, 25);
        let f = marked_form();
        let buf = flatten(render(&f, screen), screen);
        let row = find_row(&buf, "Sound").expect("sound row");
        let s_at = col_of(&buf, row, b'S');
        let hot = Theme::DEFAULT.hotkey.expect("default marks hotkeys");
        // Field 0 is focused, so the accelerator uses the selected variant.
        assert_eq!(buf.get(s_at, row).attr, hot.selected);
        // ...and the letter beside it does not.
        assert_ne!(buf.get(s_at + 1, row).attr, hot.selected);
    }

    #[test]
    fn a_button_marks_its_implicit_accelerator() {
        let screen = Size::new(80, 25);
        let buf = flatten(render(&marked_form(), screen), screen);
        let row = find_row(&buf, "OK").expect("button row");
        let o_at = col_of(&buf, row, b'O');
        let hot = Theme::DEFAULT.hotkey.expect("default marks hotkeys");
        // The OK button is not focused here, so it takes the normal variant.
        assert_eq!(buf.get(o_at, row).attr, hot.normal);
    }

    #[test]
    fn clearing_the_hotkey_theme_leaves_labels_unmarked_but_still_readable() {
        let screen = Size::new(80, 25);
        let theme = Theme {
            hotkey: None,
            ..Theme::DEFAULT
        };
        let f = marked_form();
        let buf = flatten(render_themed(&f, screen, theme).0, screen);
        let row = find_row(&buf, "Sound").expect("still reads as Sound");
        let text = row_text(&buf, row);
        assert!(!text.contains('~'), "markers are stripped either way");
        let s_at = col_of(&buf, row, b'S');
        // Every cell of the label shares one attribute: nothing is promised.
        assert_eq!(buf.get(s_at, row).attr, buf.get(s_at + 1, row).attr);
    }

    fn text_form(initial: &str, cursor: usize) -> FormState<TestOp> {
        FormState::new(
            "T",
            alloc::vec![Field {
                label: "Name",
                kind: FieldKind::Text {
                    buffer: initial.to_string(),
                    cursor,
                    selected: false,
                    overtype: false,
                    max_len: 64,
                    commit: |_| TestOp::A,
                },
                restore: alloc::vec![],
            }],
        )
    }

    #[test]
    fn a_text_field_renders_bracketed_like_other_editable_values() {
        let screen = Size::new(80, 25);
        let f = text_form("Loki", 4);
        let buf = flatten(render(&f, screen), screen);
        let row = find_row(&buf, "Name").expect("name row");
        assert!(
            row_text(&buf, row).contains("[Loki"),
            "got: {}",
            row_text(&buf, row)
        );
    }

    #[test]
    fn a_text_field_shorter_than_its_slot_does_not_scroll() {
        assert_eq!(text_scroll(0), 0);
        assert_eq!(text_scroll(TEXT_VISIBLE_W as usize - 1), 0);
    }

    #[test]
    fn a_long_text_field_scrolls_to_keep_the_caret_in_view() {
        // One past the last visible column must scroll by exactly one.
        assert_eq!(text_scroll(TEXT_VISIBLE_W as usize), 1);
        assert_eq!(text_scroll(TEXT_VISIBLE_W as usize + 5), 6);
    }

    #[test]
    fn the_caret_of_a_scrolled_text_field_stays_inside_the_brackets() {
        let screen = Size::new(80, 25);
        let long = "x".repeat(TEXT_VISIBLE_W as usize + 10);
        let caret_at = long.chars().count();
        let f = text_form(&long, caret_at);
        let (_, cursor) = render_with_cursor(&f, screen);
        let c = cursor.expect("an actively-edited text field has a caret");

        let buf = flatten(render(&f, screen), screen);
        let row = find_row(&buf, "Name").expect("name row");
        let open = col_of(&buf, row, b'[');
        let close = col_of(&buf, row, b']');
        assert!(
            c.col > open && c.col <= close,
            "caret at {} escaped the brackets at {}..{}",
            c.col,
            open,
            close
        );
    }

    #[test]
    fn a_scrolled_text_field_shows_the_end_of_the_buffer_not_the_start() {
        let screen = Size::new(80, 25);
        let long: alloc::string::String = ('a'..='z').collect();
        let caret_at = long.chars().count();
        let f = text_form(&long, caret_at);
        let buf = flatten(render(&f, screen), screen);
        let row = find_row(&buf, "Name").expect("name row");
        let text = row_text(&buf, row);
        assert!(text.contains('z'), "the caret end must be visible: {text}");
        assert!(
            !text.contains("[abc"),
            "should have scrolled past the start: {text}"
        );
    }

    fn form() -> FormState<TestOp> {
        FormState::new(
            "SETTINGS",
            alloc::vec![
                Field {
                    label: "Colour",
                    kind: FieldKind::Choice {
                        options: alloc::vec![
                            ChoiceOption {
                                label: "Red".to_string(),
                                action: TestOp::A
                            },
                            ChoiceOption {
                                label: "Green".to_string(),
                                action: TestOp::B
                            },
                        ],
                        selected: Some(0),
                    },
                    restore: alloc::vec![TestOp::A],
                },
                Field {
                    label: "Strategy",
                    kind: FieldKind::ReadOnly("Fixed".to_string()),
                    restore: Vec::new(),
                },
                Field {
                    label: "",
                    kind: FieldKind::Button {
                        label: "~O~K",
                        role: ButtonRole::Accept,
                        action: None,
                        default: true
                    },
                    restore: Vec::new(),
                },
                Field {
                    label: "",
                    kind: FieldKind::Button {
                        label: "~C~ancel",
                        role: ButtonRole::Reject,
                        action: None,
                        default: false
                    },
                    restore: Vec::new(),
                },
            ],
        )
    }

    /// Flatten the returned layers onto a blank screen so tests can assert on
    /// what the user would actually see.
    fn flatten(layers: Vec<Layer>, screen: Size) -> CellBuffer {
        let base = CellBuffer::new(screen.w, screen.h);
        let mut stack = LayerStack::new();
        for l in layers {
            stack.push(l);
        }
        let mut out = CellBuffer::new(screen.w, screen.h);
        stack.composite(&base, &mut out);
        out
    }

    /// Column of the first cell in `row` holding `ch`.
    ///
    /// Scans cells rather than searching `row_text`. A rendered row contains
    /// CP437 bytes above 0x7F — the panel border, for one — which widen to
    /// multi-byte chars in a `String`, so a byte offset from `str::find` is
    /// not a column.
    fn col_of(buf: &CellBuffer, row: u16, ch: u8) -> u16 {
        (0..buf.cols)
            .find(|&c| buf.get(c, row).ch == ch)
            .unwrap_or_else(|| panic!("no {:?} in row {row}", ch as char))
    }

    fn row_text(buf: &CellBuffer, row: u16) -> alloc::string::String {
        (0..buf.cols)
            .map(|c| buf.get(c, row).ch as char)
            .collect::<alloc::string::String>()
    }

    fn find_row(buf: &CellBuffer, needle: &str) -> Option<u16> {
        (0..buf.rows).find(|r| row_text(buf, *r).contains(needle))
    }

    #[test]
    fn panel_is_framed_with_double_line_corners() {
        let screen = Size::new(80, 25);
        let buf = flatten(render(&form(), screen), screen);
        let top = find_row(&buf, "SETTINGS").expect("title row");
        let left = (0..80).find(|c| buf.get(*c, top).ch == BoxChars::DOUBLE.tl);
        assert!(
            left.is_some(),
            "top-left corner 0xC9 present on the title row"
        );
        let l = left.unwrap();
        let right = (l + 1..80).find(|c| buf.get(*c, top).ch == BoxChars::DOUBLE.tr);
        assert!(right.is_some(), "top-right corner 0xBB present");
    }

    #[test]
    fn title_appears_in_the_top_edge() {
        let screen = Size::new(80, 25);
        let buf = flatten(render(&form(), screen), screen);
        assert!(find_row(&buf, "SETTINGS").is_some());
    }

    #[test]
    fn every_field_label_and_value_is_rendered() {
        let screen = Size::new(80, 25);
        let buf = flatten(render(&form(), screen), screen);
        assert!(find_row(&buf, "Colour").is_some());
        assert!(
            find_row(&buf, "Red").is_some(),
            "the selected option is shown"
        );
        assert!(find_row(&buf, "Strategy").is_some());
        assert!(find_row(&buf, "Fixed").is_some());
        assert!(find_row(&buf, "OK").is_some());
    }

    #[test]
    fn the_focused_row_carries_the_selection_attribute_and_others_do_not() {
        let screen = Size::new(80, 25);
        let buf = flatten(render(&form(), screen), screen);
        let focused = find_row(&buf, "Colour").expect("focused row");
        let other = find_row(&buf, "Strategy").expect("unfocused row");
        let attr_at = |row: u16| {
            let c = (0..80).find(|c| buf.get(*c, row).ch != b' ').unwrap();
            buf.get(c + 2, row).attr
        };
        assert_eq!(attr_at(focused), Theme::DEFAULT.selected);
        assert_ne!(attr_at(other), Theme::DEFAULT.selected);
    }

    #[test]
    fn the_panel_casts_a_shadow_that_does_not_cover_itself() {
        let screen = Size::new(80, 25);
        let buf = flatten(render(&form(), screen), screen);
        let title = find_row(&buf, "SETTINGS").expect("title row");
        let left = (0..80)
            .find(|c| buf.get(*c, title).ch == BoxChars::DOUBLE.tl)
            .unwrap();
        let right = (left + 1..80)
            .find(|c| buf.get(*c, title).ch == BoxChars::DOUBLE.tr)
            .unwrap();
        // One column right of the frame, one row down, is shaded.
        assert_eq!(
            buf.get(right + 1, title + 1).attr,
            neovision_core::SHADE_ATTR
        );
        // The frame itself is not shaded.
        assert_ne!(buf.get(right, title).attr, neovision_core::SHADE_ATTR);
    }

    #[test]
    fn an_open_popup_lists_every_option_below_its_field() {
        let screen = Size::new(80, 25);
        let mut f = form();
        f.handle(FormEvent::Enter);
        let buf = flatten(render(&f, screen), screen);
        let field_row = find_row(&buf, "Colour").expect("field row");
        let green = find_row(&buf, "Green").expect("popup lists the unselected option");
        assert!(green > field_row, "the popup sits below its field");
    }

    #[test]
    fn a_popup_longer_than_the_screen_scrolls_rather_than_overflowing() {
        let screen = Size::new(80, 25);
        let options: Vec<ChoiceOption<TestOp>> = (0..40)
            .map(|i| ChoiceOption {
                label: alloc::format!("Option {i}"),
                action: TestOp::A,
            })
            .collect();
        let mut f = FormState::new(
            "LONG",
            alloc::vec![Field {
                label: "Many",
                kind: FieldKind::Choice {
                    options,
                    selected: Some(0)
                },
                restore: Vec::new(),
            }],
        );
        f.handle(FormEvent::Enter);
        // Must not panic, and must stay inside the screen.
        let layers = render(&f, screen);
        for l in &layers {
            assert!(l.bounds().bottom() <= screen.h, "layer stays on screen");
            assert!(l.bounds().right() <= screen.w, "layer stays on screen");
        }
    }

    #[test]
    fn rendering_a_form_wider_than_the_screen_does_not_panic() {
        let screen = Size::new(20, 8);
        let layers = render(&form(), screen);
        for l in &layers {
            assert!(l.bounds().right() <= screen.w);
            assert!(l.bounds().bottom() <= screen.h);
        }
    }

    /// Build a form with a single Number field ("Cycle period", seeded
    /// "120") plus a trailing OK button, with the edit state fully
    /// controllable so cursor/selection rendering tests don't have to drive
    /// `FormEvent`s through the model to reach a given state.
    fn number_form_state(
        focused: bool,
        selected: bool,
        overtype: bool,
        cursor: usize,
    ) -> FormState<TestOp> {
        let mut f = FormState::new(
            "T",
            alloc::vec![
                Field {
                    label: "Cycle period",
                    kind: FieldKind::Number {
                        value: 120,
                        buffer: alloc::string::String::from("120"),
                        cursor,
                        selected,
                        overtype,
                        min: 10,
                        max: 3600,
                        unit: "s",
                        commit: |_| TestOp::A,
                    },
                    restore: Vec::new(),
                },
                Field {
                    label: "",
                    kind: FieldKind::Button {
                        label: "~O~K",
                        role: ButtonRole::Accept,
                        action: None,
                        default: true
                    },
                    restore: Vec::new()
                },
            ],
        );
        f.set_focus(if focused { 0 } else { 1 });
        f
    }

    fn number_form(focused: bool) -> FormState<TestOp> {
        number_form_state(focused, false, false, 3)
    }

    /// Build a form with a single Number field labelled "N" (distinct from
    /// `number_form_state`'s "Cycle period", so `find_row(&buf, "N")` in the
    /// fixed-width/cursor-derivation tests below can't accidentally match the
    /// other helper's rows), seeded with an arbitrary `digits` string rather
    /// than the fixed "120", plus a trailing OK button.
    fn number_form_state_buf(
        focused: bool,
        selected: bool,
        overtype: bool,
        digits: &str,
    ) -> FormState<TestOp> {
        number_form_state_buf_cur(focused, selected, overtype, digits, digits.len())
    }

    /// As `number_form_state_buf`, but with an explicit cursor index into
    /// `digits` (rather than defaulting to the end of the buffer).
    fn number_form_state_buf_cur(
        focused: bool,
        selected: bool,
        overtype: bool,
        digits: &str,
        cursor: usize,
    ) -> FormState<TestOp> {
        let mut f = FormState::new(
            "T",
            alloc::vec![
                Field {
                    label: "N",
                    kind: FieldKind::Number {
                        value: 120,
                        buffer: alloc::string::String::from(digits),
                        cursor,
                        selected,
                        overtype,
                        min: 10,
                        max: 3600,
                        unit: "s",
                        commit: |_| TestOp::A,
                    },
                    restore: Vec::new(),
                },
                Field {
                    label: "",
                    kind: FieldKind::Button {
                        label: "~O~K",
                        role: ButtonRole::Accept,
                        action: None,
                        default: true
                    },
                    restore: Vec::new(),
                },
            ],
        );
        f.set_focus(if focused { 0 } else { 1 });
        f
    }

    /// Column of `needle` on `row`, as a cell index rather than a byte
    /// offset.
    ///
    /// `str::find` returns a *byte* offset, which is not the same thing here:
    /// the panel border draws `º` (VGA 0xBA, the double-line vertical, cast
    /// to `char`) to the left of every field row, and that single char is 2
    /// UTF-8 bytes. Any test that used `row_text(..).find(needle) as u16` to
    /// index back into `buf` would be off by one for every such border char
    /// it crosses — `chars().position()` counts scalar values, matching how
    /// `row_text` built the string one char per cell.
    fn column_of(buf: &CellBuffer, row: u16, needle: char) -> u16 {
        row_text(buf, row)
            .chars()
            .position(|c| c == needle)
            .expect("needle present on row") as u16
    }

    /// Attributes of the digit cells right after `[` on the number row —
    /// i.e. what `value_column`'s seeded "120" buffer occupies.
    fn digit_cell_attrs(buf: &CellBuffer, row: u16) -> Vec<u8> {
        let mut x = column_of(buf, row, '[') + 1;
        let mut attrs = Vec::new();
        while buf.get(x, row).ch.is_ascii_digit() {
            attrs.push(buf.get(x, row).attr);
            x += 1;
        }
        attrs
    }

    #[test]
    fn a_number_field_brackets_only_the_digits_and_shows_the_range_outside() {
        let screen = Size::new(80, 25);
        let buf = flatten(render(&number_form(false), screen), screen);
        let row = find_row(&buf, "Cycle period").expect("number row");
        let text = row_text(&buf, row);
        let open = text.find('[').unwrap();
        let close = text.find(']').unwrap();
        let val = text.find("120").unwrap();
        let range = text.find("(10-3600)").expect("range shown");
        assert!(
            open < val && val < close,
            "digits are inside the brackets: {text}"
        );
        assert!(range > close, "the range sits OUTSIDE the brackets: {text}");
    }

    #[test]
    fn a_focused_number_field_yields_a_cursor_and_an_unfocused_one_does_not() {
        // A focused, editing (not selected) field yields an Insert-shape
        // `TextCursor` descriptor rather than the old glyph drawn straight
        // into the cell buffer (which overwrote the digit under it — the bug
        // this replacement kills, see the module doc on `render_with_cursor`).
        let (_layers, cursor) = render_with_cursor(&number_form(false), Size::new(80, 25));
        assert!(
            cursor.is_none(),
            "an unfocused Number field yields no cursor"
        );

        let (_layers, cursor) = render_with_cursor(&number_form(true), Size::new(80, 25));
        assert_eq!(
            cursor
                .expect("a focused, editing Number field yields a cursor")
                .shape,
            CursorShape::Insert,
            "a focused, editing Number field yields an Insert-shape cursor"
        );
    }

    #[test]
    fn a_selected_number_field_highlights_the_whole_value() {
        let f = number_form_state(/* focused */ true, /* selected */ true, false, 3);
        let screen = Size::new(80, 25);
        let buf = flatten(render(&f, screen), screen);
        let row = find_row(&buf, "Cycle period").expect("number row");
        let sel = Theme::DEFAULT.selection;
        let digits = digit_cell_attrs(&buf, row);
        assert_eq!(digits.len(), 3, "all three seeded digits are present");
        assert!(
            digits.iter().all(|&a| a == sel),
            "selected digits use theme.selection"
        );
    }

    #[test]
    fn an_insert_cursor_is_insert_shape_and_overtype_is_overtype_shape() {
        let ins = number_form_state(true, false, false, 1); // editing, cursor at 1
        let ovr = number_form_state(true, false, true, 1);
        let (_l, cursor) = render_with_cursor(&ins, Size::new(80, 25));
        assert_eq!(
            cursor.unwrap().shape,
            CursorShape::Insert,
            "insert mode yields an Insert-shape cursor"
        );
        let (_l, cursor) = render_with_cursor(&ovr, Size::new(80, 25));
        assert_eq!(
            cursor.unwrap().shape,
            CursorShape::Overtype,
            "overtype mode yields an Overtype-shape cursor"
        );
    }

    #[test]
    fn a_focused_emptied_number_field_shows_no_stale_digits() {
        // Focused, NOT selected, buffer emptied (select -> Backspace): must
        // render as genuinely empty ([ ]-style, cursor at column 0) rather
        // than falling back to the stale committed `value` under the edit
        // cursor.
        let mut f = FormState::new(
            "T",
            alloc::vec![
                Field {
                    label: "Cycle period",
                    kind: FieldKind::Number {
                        value: 120,
                        buffer: alloc::string::String::new(),
                        cursor: 0,
                        selected: false,
                        overtype: false,
                        min: 10,
                        max: 3600,
                        unit: "s",
                        commit: |_| TestOp::A,
                    },
                    restore: Vec::new(),
                },
                Field {
                    label: "",
                    kind: FieldKind::Button {
                        label: "~O~K",
                        role: ButtonRole::Accept,
                        action: None,
                        default: true
                    },
                    restore: Vec::new(),
                },
            ],
        );
        f.set_focus(0);
        let screen = Size::new(80, 25);
        let buf = flatten(render(&f, screen), screen);
        let row = find_row(&buf, "Cycle period").expect("number row");
        let text = row_text(&buf, row);
        assert!(
            !text.contains("120"),
            "the stale committed value must not appear: {text}"
        );
        let open = column_of(&buf, row, '[');
        let (_layers, cursor) = render_with_cursor(&f, screen);
        assert_eq!(
            cursor
                .expect("a focused, editing (empty-buffer) field still yields a cursor")
                .col,
            open + 1,
            "the insert cursor sits at column 0, right after '[': {text}"
        );
    }

    #[test]
    fn an_unfocused_number_shows_no_cursor_or_selection() {
        let f = number_form_state(false, false, false, 3);
        let (_layers, cursor) = render_with_cursor(&f, Size::new(80, 25));
        assert!(cursor.is_none(), "unfocused field yields no cursor");
    }

    #[test]
    fn a_screen_too_short_for_a_popup_draws_none_rather_than_overhanging() {
        let mut f = form();
        f.handle(FormEvent::Enter);
        for h in 0..=3u16 {
            let screen = Size::new(20, h);
            for l in render(&f, screen) {
                assert!(
                    l.bounds().bottom() <= screen.h,
                    "layer overhangs a {h}-row screen"
                );
                assert!(l.bounds().right() <= screen.w);
            }
        }
    }

    // --- Field chrome, button row, separator ---

    #[test]
    fn an_editable_fields_value_is_bracketed() {
        let screen = Size::new(80, 25);
        let buf = flatten(render(&form(), screen), screen);
        let row = find_row(&buf, "Colour").expect("colour row");
        let text = row_text(&buf, row);
        let value_at = text.find("Red").expect("the selected value is shown");
        let open = text.find('[').expect("opening bracket present");
        let close = text.find(']').expect("closing bracket present");
        assert!(
            open < value_at && value_at < close,
            "the value sits inside the brackets: {text}"
        );
    }

    #[test]
    fn a_read_only_fields_value_is_not_bracketed() {
        let screen = Size::new(80, 25);
        let buf = flatten(render(&form(), screen), screen);
        let row = find_row(&buf, "Fixed").expect("strategy row");
        let text = row_text(&buf, row);
        assert!(
            !text.contains('[') && !text.contains(']'),
            "a ReadOnly value must not get field chrome: {text}"
        );
    }

    #[test]
    fn ok_and_cancel_share_a_row_in_that_order() {
        let screen = Size::new(80, 25);
        let buf = flatten(render(&form(), screen), screen);
        let ok_row = find_row(&buf, "OK").expect("OK is drawn");
        let cancel_row = find_row(&buf, "Cancel").expect("Cancel is drawn");
        assert_eq!(ok_row, cancel_row, "OK and Cancel share a single row");
        let text = row_text(&buf, ok_row);
        let ok_at = text.find("OK").unwrap();
        let cancel_at = text.find("Cancel").unwrap();
        assert!(ok_at < cancel_at, "OK appears before Cancel: {text}");
    }

    #[test]
    fn the_focused_button_carries_the_selected_attribute_and_the_unfocused_one_does_not() {
        let screen = Size::new(80, 25);
        let mut f = form();
        f.set_focus(2); // OK
        let buf = flatten(render(&f, screen), screen);
        let row = find_row(&buf, "OK").expect("button row");
        let text = row_text(&buf, row);
        let ok_at = text.find("OK").unwrap() as u16;
        let cancel_at = text.find("Cancel").unwrap() as u16;
        assert_eq!(
            buf.get(ok_at, row).attr,
            Theme::DEFAULT.selected,
            "the focused button's label is highlighted"
        );
        // The requirement is the WHOLE chrome, not just the label: check the
        // opening bracket two cells before "OK" (button_chrome("OK") is
        // "[  OK  ]", so the bracket sits 3 cells before the label).
        assert_eq!(
            buf.get(ok_at - 3, row).attr,
            Theme::DEFAULT.selected,
            "the focused button's bracket is highlighted too, not just its label"
        );
        assert_ne!(
            buf.get(cancel_at, row).attr,
            Theme::DEFAULT.selected,
            "the unfocused button must not be highlighted"
        );
    }

    #[test]
    fn an_oversized_value_is_truncated_and_does_not_overwrite_the_frame() {
        let screen = Size::new(80, 25);
        let long = "X".repeat(30);
        let f = FormState::new(
            "SETTINGS",
            alloc::vec![Field {
                label: "Long",
                kind: FieldKind::Choice {
                    options: alloc::vec![ChoiceOption {
                        label: long.clone(),
                        action: TestOp::A,
                    }],
                    selected: Some(0),
                },
                restore: Vec::new(),
            }],
        );
        let buf = flatten(render(&f, screen), screen);
        let top = find_row(&buf, "SETTINGS").expect("title row");
        let left = (0..80)
            .find(|c| buf.get(*c, top).ch == BoxChars::DOUBLE.tl)
            .unwrap();
        let right = (left + 1..80)
            .find(|c| buf.get(*c, top).ch == BoxChars::DOUBLE.tr)
            .unwrap();
        let row = top + 1; // the form's only field
                           // The right border survives untouched...
        assert_eq!(
            buf.get(right, row).ch,
            BoxChars::DOUBLE.v,
            "an oversized value must not overwrite the right border"
        );
        let text = row_text(&buf, row);
        // ...and the value itself is cut short rather than overflowing.
        assert!(text.contains(']'), "the bracket still closes: {text}");
        assert!(
            !text.contains(long.as_str()),
            "the full 30-char value must not appear verbatim: {text}"
        );
    }

    #[test]
    fn the_separator_row_has_tee_glyphs_at_the_frame_columns() {
        let screen = Size::new(80, 25);
        let buf = flatten(render(&form(), screen), screen);
        let top = find_row(&buf, "SETTINGS").expect("title row");
        let left = (0..80)
            .find(|c| buf.get(*c, top).ch == BoxChars::DOUBLE.tl)
            .unwrap();
        let right = (left + 1..80)
            .find(|c| buf.get(*c, top).ch == BoxChars::DOUBLE.tr)
            .unwrap();
        // `form()` has 2 non-button fields (Colour, Strategy) before the
        // separator, which sits on the row right after them.
        let sep_row = top + 2 + 1;
        assert_eq!(
            buf.get(left, sep_row).ch,
            BoxChars::DOUBLE.tee_l,
            "left tee meets the frame"
        );
        assert_eq!(
            buf.get(right, sep_row).ch,
            BoxChars::DOUBLE.tee_r,
            "right tee meets the frame"
        );
        assert_eq!(
            buf.get(left + 1, sep_row).ch,
            BoxChars::DOUBLE.h,
            "the rule between the tees uses the double-line horizontal glyph"
        );
    }

    // --- TextCursor derivation + fixed-width entry field ---

    #[test]
    fn number_field_is_fixed_width_across_digit_counts() {
        let screen = Size::new(80, 25);
        let close_col = |digits: &str| -> usize {
            let f = number_form_state_buf(false, false, false, digits);
            let buf = flatten(render(&f, screen), screen);
            let row = find_row(&buf, "N").expect("number row");
            let text = row_text(&buf, row);
            text.chars()
                .position(|c| c == ']')
                .expect("closing bracket")
        };
        // The ']' column must not move as the digit count changes.
        assert_eq!(
            close_col("1"),
            close_col("1204"),
            "field width must be stable"
        );
        assert_eq!(
            close_col("120"),
            close_col("1"),
            "field width must be stable"
        );
    }

    #[test]
    fn cursor_descriptor_tracks_focused_editing_field() {
        let screen = Size::new(80, 25);
        // Focused, not selected, insert mode, cursor at index 1 in "120".
        let f = number_form_state_buf_cur(true, false, false, "120", 1);
        let (_layers, cursor) = render_with_cursor(&f, screen);
        let c = cursor.expect("focused editing field yields a cursor");
        assert_eq!(c.shape, neovision_core::CursorShape::Insert);
        // Cursor sits at the value column + '[' + index within the digits.
        // Recompute expected from the same rendered layout:
        let buf = flatten(render(&f, screen), screen);
        let row = find_row(&buf, "N").expect("number row");
        let open = {
            let text = row_text(&buf, row);
            text.chars().position(|ch| ch == '[').unwrap()
        };
        assert_eq!(c.row as usize, row as usize, "cursor on the field's row");
        assert_eq!(c.col as usize, open + 1 + 1, "cursor after '[' + index 1");
    }

    #[test]
    fn overtype_field_yields_block_shape() {
        let f = number_form_state_buf_cur(true, false, true, "120", 0);
        let (_l, cursor) = render_with_cursor(&f, Size::new(80, 25));
        assert_eq!(cursor.unwrap().shape, neovision_core::CursorShape::Overtype);
    }

    #[test]
    fn no_cursor_when_selected_or_unfocused() {
        let sel = number_form_state_buf_cur(true, true, false, "120", 3); // selected
        let unf = number_form_state_buf_cur(false, false, false, "120", 3); // unfocused
        assert!(
            render_with_cursor(&sel, Size::new(80, 25)).1.is_none(),
            "selected -> no cursor"
        );
        assert!(
            render_with_cursor(&unf, Size::new(80, 25)).1.is_none(),
            "unfocused -> no cursor"
        );
    }
}
