//! A pixel-framebuffer host for neovision.
//!
//! The companion to `terminal.rs`. That host hands cells to a terminal and lets
//! the terminal find glyphs and colours; this one rasterizes every pixel
//! itself, which is what a wasm canvas, an embedded display, or a real VESA
//! mode would do. It owes the toolkit the same three things:
//!
//! 1. **Glyphs.** A cell holds a CP437 byte. [`font::glyph`] returns its 16×8
//!    bitmap, and [`blit_cell`] walks the bits.
//! 2. **Colour.** A cell holds a VGA attribute byte. [`PALETTE`] turns each
//!    nibble into RGB.
//! 3. **Input.** The toolkit speaks [`FormEvent`]; [`to_form_event`] translates.
//!
//! Run it:
//!
//! ```console
//! cargo run --example framebuffer              # a window
//! cargo run --example framebuffer -- --single  # one PPM frame
//! ```
//!
//! `--single` writes a frame and exits, needing no display at all, so the
//! rasterizer stays verifiable in CI where no window can open.

use std::io::{self, Write};

use minifb::{Key, KeyRepeat, Scale, Window, WindowOptions};

use neovision::neovision_core::font;
use neovision::{
    render_with_cursor, Cell, CellBuffer, CellDraw, ChoiceOption, ClusterItem, ClusterStyle,
    CursorShape, EnterReach, Field, FieldKind, FormEvent, FormState, LayerStack, Point, Size,
    TextCursor, Theme,
};

/// The 16 VGA colours as `0x00RRGGBB`, in hardware order.
///
/// Background nibbles only carry three bits — bit 7 is blink — so backgrounds
/// come from the first eight entries and foregrounds from all sixteen.
const PALETTE: [u32; 16] = [
    0x00_000000, // 0 black
    0x00_0000AA, // 1 blue
    0x00_00AA00, // 2 green
    0x00_00AAAA, // 3 cyan
    0x00_AA0000, // 4 red
    0x00_AA00AA, // 5 magenta
    0x00_AA5500, // 6 brown
    0x00_AAAAAA, // 7 light grey
    0x00_555555, // 8 dark grey
    0x00_5555FF, // 9 bright blue
    0x00_55FF55, // A bright green
    0x00_55FFFF, // B bright cyan
    0x00_FF5555, // C bright red
    0x00_FF55FF, // D bright magenta
    0x00_FFFF55, // E yellow
    0x00_FFFFFF, // F white
];

/// What this demo's form can ask for. neovision stores these and never looks
/// inside them; the host alone decides what they mean.
#[derive(Debug, Clone, PartialEq, Eq)]
#[allow(clippy::enum_variant_names)] // They really are all setters.
enum Action {
    SetTheme(&'static str),
    SetScale(&'static str),
    SetScanlines(bool),
    SetDelay(u32),
    SetName(String),
}

// ---------------------------------------------------------------------------
// Rasterizing
// ---------------------------------------------------------------------------

/// A pixel buffer, sized in whole character cells.
///
/// Pixels are **palette indices**, not RGB, because that is what the hardware
/// being imitated actually stored. Keeping them indexed until the last moment
/// means the window, the PPM and the GIF each convert once, and the GIF needs
/// no quantization at all — its native model is exactly this.
struct Frame {
    px: Vec<u8>,
    w: usize,
    h: usize,
}

impl Frame {
    fn for_grid(cols: u16, rows: u16) -> Self {
        let w = cols as usize * font::GLYPH_W as usize;
        let h = rows as usize * font::GLYPH_H as usize;
        Self {
            px: vec![0; w * h],
            w,
            h,
        }
    }

    #[inline]
    fn set(&mut self, x: usize, y: usize, colour: u8) {
        if x < self.w && y < self.h {
            self.px[y * self.w + x] = colour;
        }
    }

    /// Resolve to the packed `0x00RRGGBB` a window wants.
    fn to_argb(&self) -> Vec<u32> {
        self.px.iter().map(|&i| PALETTE[i as usize]).collect()
    }
}

/// Draw one cell's glyph: set bits take the foreground colour, clear bits the
/// background. This is the whole of "how a cell becomes pixels".
fn blit_cell(frame: &mut Frame, col: u16, row: u16, cell: Cell) {
    let fg = cell.fg();
    let bg = cell.bg();
    let glyph = font::glyph(cell.ch);
    let ox = col as usize * font::GLYPH_W as usize;
    let oy = row as usize * font::GLYPH_H as usize;

    for (y, &bits) in glyph.iter().enumerate() {
        for x in 0..font::GLYPH_W as usize {
            // Bit 7 is the leftmost pixel.
            let lit = (bits >> (font::GLYPH_W as usize - 1 - x)) & 1 != 0;
            frame.set(ox + x, oy + y, if lit { fg } else { bg });
        }
    }
}

/// Draw the caret.
///
/// This is where a pixel host has to decide something a terminal host never
/// does: what colour a caret is. [`TextCursor`] carries position and shape but
/// deliberately not colour, so the two shapes resolve it differently.
///
/// `Overtype` is documented as reverse video, so it needs nothing external —
/// swapping the cell's own two colours is the whole of it, and matches what VGA
/// hardware does. `Insert` has no such rule, so it takes its colour from the
/// theme, which is what `Theme::cursor` is for: a skin can recolour the
/// caret without touching the renderer.
fn draw_caret(frame: &mut Frame, screen: &CellBuffer, cursor: TextCursor, theme: Theme) {
    let cell = screen.get(cursor.col, cursor.row);
    let ox = cursor.col as usize * font::GLYPH_W as usize;
    let oy = cursor.row as usize * font::GLYPH_H as usize;

    match cursor.shape {
        CursorShape::Overtype => {
            let fg = cell.fg();
            let bg = cell.bg();
            for y in 0..font::GLYPH_H as usize {
                for x in 0..font::GLYPH_W as usize {
                    let lit = font::pixel(cell.ch, x as u16, y as u16);
                    // Reverse video: the glyph keeps its shape, the two
                    // colours trade places.
                    frame.set(ox + x, oy + y, if lit { bg } else { fg });
                }
            }
        }
        CursorShape::Insert => {
            let colour = theme.cursor & 0x0F;
            // A two-scanline underline on the cell's bottom rows.
            for y in (font::GLYPH_H as usize - 2)..font::GLYPH_H as usize {
                for x in 0..font::GLYPH_W as usize {
                    frame.set(ox + x, oy + y, colour);
                }
            }
        }
    }
}

fn rasterize(screen: &CellBuffer, cursor: Option<TextCursor>, theme: Theme) -> Frame {
    let mut frame = Frame::for_grid(screen.cols, screen.rows);
    for row in 0..screen.rows {
        for col in 0..screen.cols {
            blit_cell(&mut frame, col, row, screen.get(col, row));
        }
    }
    if let Some(c) = cursor {
        draw_caret(&mut frame, screen, c, theme);
    }
    frame
}

// ---------------------------------------------------------------------------
// Input
// ---------------------------------------------------------------------------

fn to_form_event(key: Key, shift: bool, alt: bool) -> Option<FormEvent> {
    // Alt+letter is this host's accelerator, and emitting it is optional: a
    // host with no Alt never sends `Hotkey` and simply has no accelerators.
    if alt {
        if let Some(c) = letter_of(key) {
            return Some(FormEvent::Hotkey(c));
        }
    }
    if let Some(c) = letter_of(key) {
        // Plain letters are text, not accelerators. A `Text` field is useless
        // without them.
        return Some(FormEvent::Char(if shift {
            c.to_ascii_uppercase()
        } else {
            c
        }));
    }
    Some(match key {
        Key::Space => FormEvent::Char(' '),
        Key::Up => FormEvent::Up,
        Key::Down => FormEvent::Down,
        Key::Left => FormEvent::Left,
        Key::Right => FormEvent::Right,
        Key::Tab if shift => FormEvent::BackTab,
        Key::Tab => FormEvent::Tab,
        Key::Enter => FormEvent::Enter,
        Key::Escape => FormEvent::Escape,
        Key::Backspace => FormEvent::Backspace,
        Key::Home => FormEvent::Home,
        Key::End => FormEvent::End,
        Key::Delete => FormEvent::Delete,
        Key::Insert => FormEvent::Insert,
        Key::Key0 | Key::NumPad0 => FormEvent::Char('0'),
        Key::Key1 | Key::NumPad1 => FormEvent::Char('1'),
        Key::Key2 | Key::NumPad2 => FormEvent::Char('2'),
        Key::Key3 | Key::NumPad3 => FormEvent::Char('3'),
        Key::Key4 | Key::NumPad4 => FormEvent::Char('4'),
        Key::Key5 | Key::NumPad5 => FormEvent::Char('5'),
        Key::Key6 | Key::NumPad6 => FormEvent::Char('6'),
        Key::Key7 | Key::NumPad7 => FormEvent::Char('7'),
        Key::Key8 | Key::NumPad8 => FormEvent::Char('8'),
        Key::Key9 | Key::NumPad9 => FormEvent::Char('9'),
        _ => return None,
    })
}

/// The letter a key stands for, for accelerator matching.
fn letter_of(key: Key) -> Option<char> {
    Some(match key {
        Key::A => 'a',
        Key::B => 'b',
        Key::C => 'c',
        Key::D => 'd',
        Key::E => 'e',
        Key::F => 'f',
        Key::G => 'g',
        Key::H => 'h',
        Key::I => 'i',
        Key::J => 'j',
        Key::K => 'k',
        Key::L => 'l',
        Key::M => 'm',
        Key::N => 'n',
        Key::O => 'o',
        Key::P => 'p',
        Key::Q => 'q',
        Key::R => 'r',
        Key::S => 's',
        Key::T => 't',
        Key::U => 'u',
        Key::V => 'v',
        Key::W => 'w',
        Key::X => 'x',
        Key::Y => 'y',
        Key::Z => 'z',
        _ => return None,
    })
}

// ---------------------------------------------------------------------------
// The form and its desktop
// ---------------------------------------------------------------------------

fn choice(label: &'static str, options: &[(&str, Action)], selected: usize) -> Field<Action> {
    let options: Vec<ChoiceOption<Action>> = options
        .iter()
        .map(|(label, action)| ChoiceOption {
            label: label.to_string(),
            action: action.clone(),
        })
        .collect();
    let restore = vec![options[selected].action.clone()];
    Field {
        label,
        kind: FieldKind::Choice {
            options,
            selected: Some(selected),
        },
        restore,
    }
}

fn demo_form() -> FormState<Action> {
    // A dialog, so Enter finishes it. The library defaults to OperateOnly,
    // where Enter only ever operates the focused control.
    FormState::new(
        " Video Settings ",
        vec![
            choice(
                "Palette",
                &[
                    ("Classic", Action::SetTheme("Classic")),
                    ("Amber", Action::SetTheme("Amber")),
                    ("Green Screen", Action::SetTheme("Green Screen")),
                ],
                0,
            ),
            choice(
                "Scale",
                &[
                    ("1x", Action::SetScale("1x")),
                    ("2x", Action::SetScale("2x")),
                    ("3x", Action::SetScale("3x")),
                ],
                1,
            ),
            Field {
                label: "Profile",
                kind: FieldKind::Text {
                    buffer: "default".to_string(),
                    cursor: "default".len(),
                    selected: true,
                    overtype: false,
                    max_len: 32,
                    commit: |s| Action::SetName(s.to_string()),
                },
                restore: vec![Action::SetName("default".to_string())],
            },
            Field {
                label: "Video",
                kind: FieldKind::Cluster {
                    style: ClusterStyle::Radio,
                    items: vec![
                        ClusterItem {
                            label: "CGA".to_string(),
                            on: true,
                            on_action: Action::SetScale("CGA"),
                            off_action: None,
                        },
                        ClusterItem {
                            label: "EGA".to_string(),
                            on: false,
                            on_action: Action::SetScale("EGA"),
                            off_action: None,
                        },
                    ],
                    cursor: 0,
                },
                restore: vec![Action::SetScale("CGA")],
            },
            Field {
                label: "Options",
                kind: FieldKind::Cluster {
                    style: ClusterStyle::Check,
                    items: vec![
                        ClusterItem {
                            label: "Scanlines".to_string(),
                            on: false,
                            on_action: Action::SetScanlines(true),
                            off_action: Some(Action::SetScanlines(false)),
                        },
                        ClusterItem {
                            label: "Blink".to_string(),
                            on: false,
                            on_action: Action::SetScanlines(true),
                            off_action: Some(Action::SetScanlines(false)),
                        },
                    ],
                    cursor: 0,
                },
                restore: vec![],
            },
            Field {
                label: "Frame delay",
                kind: FieldKind::Number {
                    value: 16,
                    buffer: "16".to_string(),
                    cursor: 2,
                    selected: true,
                    overtype: false,
                    min: 0,
                    max: 9999,
                    unit: "ms",
                    commit: Action::SetDelay,
                },
                restore: vec![Action::SetDelay(16)],
            },
            Field {
                label: "Renderer",
                kind: FieldKind::ReadOnly("framebuffer 8x16".to_string()),
                restore: vec![],
            },
            Field::ok(),
            Field::cancel(),
        ],
    )
    .with_enter_reach(EnterReach::AcceptWhenIdle)
}

fn desktop(size: Size, status: &str) -> CellBuffer {
    let mut buf = CellBuffer::new(size.w, size.h);
    buf.fill(Cell::new(0xB0, 0x08, 0x01, false));

    buf.fill_row(0, Cell::new(b' ', 0x0, 0x3, false));
    buf.write_str(Point::new(2, 0), "neovision · framebuffer host", 0x30);

    let footer = size.h.saturating_sub(1);
    buf.fill_row(footer, Cell::new(b' ', 0x0, 0x3, false));
    buf.write_str(Point::new(2, footer), status, 0x30);

    buf
}

/// Compose the desktop, the dialog layers, and the caret into one cell buffer.
fn compose(
    state: &FormState<Action>,
    screen: Size,
    status: &str,
) -> (CellBuffer, Option<TextCursor>) {
    let base = desktop(screen, status);
    let (layers, cursor) = render_with_cursor(state, screen);

    let mut stack = LayerStack::new();
    for layer in layers {
        stack.push(layer);
    }
    let mut composed = CellBuffer::new(screen.w, screen.h);
    stack.composite(&base, &mut composed);

    (composed, cursor)
}

// ---------------------------------------------------------------------------
// Output
// ---------------------------------------------------------------------------

/// Write the frame as a binary PPM (P6), which every image viewer reads and
/// which needs no encoder dependency.
fn write_ppm(frame: &Frame, path: &str) -> io::Result<()> {
    let mut out = io::BufWriter::new(std::fs::File::create(path)?);
    write!(out, "P6\n{} {}\n255\n", frame.w, frame.h)?;
    let mut rgb = Vec::with_capacity(frame.px.len() * 3);
    for &i in &frame.px {
        let c = PALETTE[i as usize];
        rgb.push((c >> 16) as u8);
        rgb.push((c >> 8) as u8);
        rgb.push(c as u8);
    }
    out.write_all(&rgb)?;
    out.flush()
}

/// The 16 VGA colours flattened to the RGB triples a GIF global palette wants.
fn gif_palette() -> Vec<u8> {
    let mut p = Vec::with_capacity(16 * 3);
    for c in PALETTE {
        p.push((c >> 16) as u8);
        p.push((c >> 8) as u8);
        p.push(c as u8);
    }
    p
}

/// Write an animated GIF, one frame per key in the script.
///
/// No quantization happens anywhere: a cell's attribute nibble *is* a palette
/// index, and a GIF is an indexed format, so the frames go out exactly as
/// rasterized. That is also why the files are small despite being 640x400.
fn write_gif(path: &str, script: &[FormEvent], delay_cs: u16) -> io::Result<()> {
    let screen = Size::new(COLS, ROWS);
    let mut state = demo_form();
    let mut file = std::fs::File::create(path)?;
    let palette = gif_palette();

    let px_w = COLS * font::GLYPH_W;
    let px_h = ROWS * font::GLYPH_H;
    let mut encoder = gif::Encoder::new(&mut file, px_w, px_h, &palette)
        .map_err(|e| io::Error::other(format!("gif header: {e}")))?;
    encoder
        .set_repeat(gif::Repeat::Infinite)
        .map_err(|e| io::Error::other(format!("gif repeat: {e}")))?;

    // One frame before any key, then one after each — and a long hold on the
    // last so a loop does not snap back the instant it finishes.
    let render = |state: &FormState<Action>| {
        let (composed, cursor) = compose(
            state,
            screen,
            "Tab/arrows · Enter edits · Alt+letter jumps · Esc quits",
        );
        rasterize(&composed, cursor, Theme::DEFAULT)
    };

    let mut prev = render(&state);
    encoder
        .write_frame(&gif::Frame {
            width: px_w,
            height: px_h,
            delay: delay_cs,
            dispose: gif::DisposalMethod::Keep,
            buffer: std::borrow::Cow::Borrowed(&prev.px),
            ..Default::default()
        })
        .map_err(|e| io::Error::other(format!("gif frame: {e}")))?;

    for (i, ev) in script.iter().enumerate() {
        state.handle(*ev);
        let next = render(&state);
        let delay = if i + 1 == script.len() {
            delay_cs * 5
        } else {
            delay_cs
        };

        // Only the changed rectangle goes out. Between two frames of a form
        // that is a row or two of a dialog rather than the whole 640x400
        // screen, and `DisposalMethod::Keep` leaves the rest standing.
        let ((left, top, w, h), buffer) = dirty_rect(&prev, &next);
        encoder
            .write_frame(&gif::Frame {
                left,
                top,
                width: w,
                height: h,
                delay,
                dispose: gif::DisposalMethod::Keep,
                buffer: std::borrow::Cow::Owned(buffer),
                ..Default::default()
            })
            .map_err(|e| io::Error::other(format!("gif frame: {e}")))?;
        prev = next;
    }
    Ok(())
}

/// The bounding box of pixels that differ between two frames, and just those
/// pixels. Falls back to a single pixel when nothing changed, since a frame
/// still has to exist to carry its delay.
fn dirty_rect(prev: &Frame, next: &Frame) -> ((u16, u16, u16, u16), Vec<u8>) {
    let (mut x0, mut y0, mut x1, mut y1) = (usize::MAX, usize::MAX, 0usize, 0usize);
    for y in 0..next.h {
        for x in 0..next.w {
            if prev.px[y * prev.w + x] != next.px[y * next.w + x] {
                x0 = x0.min(x);
                y0 = y0.min(y);
                x1 = x1.max(x);
                y1 = y1.max(y);
            }
        }
    }
    if x0 == usize::MAX {
        return ((0, 0, 1, 1), vec![next.px[0]]);
    }

    let (w, h) = (x1 - x0 + 1, y1 - y0 + 1);
    let mut buf = Vec::with_capacity(w * h);
    for y in y0..=y1 {
        buf.extend_from_slice(&next.px[y * next.w + x0..y * next.w + x1 + 1]);
    }
    ((x0 as u16, y0 as u16, w as u16, h as u16), buf)
}

const COLS: u16 = 80;
const ROWS: u16 = 25;

/// The scripted interaction the README's animation shows: pick a palette from
/// a popup, flip a toggle, type into the number field, then press OK.
const README_SCRIPT: &[FormEvent] = &[
    FormEvent::Enter, // open the Palette dropdown
    FormEvent::Down,  // highlight Amber
    FormEvent::Enter, // choose it
    FormEvent::Tab,   // -> Scale
    FormEvent::Tab,   // -> Profile, whole-selected on entry
    FormEvent::Char('n'),
    FormEvent::Char('e'),
    FormEvent::Char('o'),
    FormEvent::Tab,       // -> Video, a radio cluster
    FormEvent::Down,      // a radio caret selects as it moves
    FormEvent::Tab,       // -> Options, a check cluster
    FormEvent::Char(' '), // Space flips the item under the caret
    FormEvent::Down,
    FormEvent::Char(' '), // ...and the next one
    FormEvent::Tab,       // -> Frame delay
    FormEvent::Char('3'),
    FormEvent::Char('3'),
    // Ends on OK focused rather than pressed: pressing it closes the form,
    // which changes nothing on screen and would spend the last frame on a
    // picture identical to the one before it.
    FormEvent::Tab, // -> OK (Renderer is read-only and skipped)
];

fn main() -> io::Result<()> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.iter().any(|a| a == "--single") {
        return single_frame(&args);
    }
    if args.iter().any(|a| a == "--gif") {
        let path = flag_value(&args, "--gif")
            .cloned()
            .unwrap_or_else(|| "demo.gif".to_string());
        let script: Vec<FormEvent> = match flag_value(&args, "--keys") {
            Some(keys) => keys
                .split(',')
                .filter(|t| !t.is_empty())
                .map(|t| parse_key(t).ok_or_else(|| io::Error::other(format!("unknown key: {t}"))))
                .collect::<io::Result<_>>()?,
            None => README_SCRIPT.to_vec(),
        };
        write_gif(&path, &script, 60)?;
        let bytes = std::fs::metadata(&path)?.len();
        println!(
            "wrote {} ({} frames, {} KiB)",
            path,
            script.len() + 1,
            bytes / 1024
        );
        return Ok(());
    }
    interactive()
}

/// Parse one `--keys` token into an event.
///
/// Lets a headless run reach any form state, so states a human would have to
/// tab into — an edit caret, an open popup — stay checkable without a display.
fn parse_key(token: &str) -> Option<FormEvent> {
    Some(match token {
        "up" => FormEvent::Up,
        "down" => FormEvent::Down,
        "left" => FormEvent::Left,
        "right" => FormEvent::Right,
        "tab" => FormEvent::Tab,
        "backtab" => FormEvent::BackTab,
        "enter" => FormEvent::Enter,
        "esc" => FormEvent::Escape,
        "backspace" => FormEvent::Backspace,
        "home" => FormEvent::Home,
        "end" => FormEvent::End,
        "delete" => FormEvent::Delete,
        "insert" => FormEvent::Insert,
        other => {
            let mut chars = other.chars();
            let c = chars.next()?;
            if chars.next().is_some() {
                return None;
            }
            FormEvent::Char(c)
        }
    })
}

fn flag_value<'a>(args: &'a [String], flag: &str) -> Option<&'a String> {
    args.iter()
        .position(|a| a == flag)
        .and_then(|i| args.get(i + 1))
        .filter(|a| !a.starts_with("--"))
}

fn single_frame(args: &[String]) -> io::Result<()> {
    let path = flag_value(args, "--single")
        .cloned()
        .unwrap_or_else(|| "frame.ppm".to_string());

    let mut state = demo_form();
    if let Some(keys) = flag_value(args, "--keys") {
        for token in keys.split(',').filter(|t| !t.is_empty()) {
            let Some(ev) = parse_key(token) else {
                return Err(io::Error::other(format!("unknown key token: {token}")));
            };
            state.handle(ev);
        }
    }

    let screen = Size::new(COLS, ROWS);
    let (composed, cursor) = compose(
        &state,
        screen,
        "Tab/arrows · Enter edits · Alt+letter jumps · Esc quits",
    );
    let frame = rasterize(&composed, cursor, Theme::DEFAULT);
    write_ppm(&frame, &path)?;
    match cursor {
        Some(c) => println!(
            "wrote {} ({}x{} px), caret at col {} row {} ({:?})",
            path, frame.w, frame.h, c.col, c.row, c.shape
        ),
        None => println!("wrote {} ({}x{} px), no caret", path, frame.w, frame.h),
    }
    Ok(())
}

fn interactive() -> io::Result<()> {
    let screen = Size::new(COLS, ROWS);
    let mut state = demo_form();
    let mut log: Vec<Action> = Vec::new();

    let px_w = COLS as usize * font::GLYPH_W as usize;
    let px_h = ROWS as usize * font::GLYPH_H as usize;

    let mut window = Window::new(
        "neovision - framebuffer host",
        px_w,
        px_h,
        WindowOptions {
            scale: Scale::X2,
            ..WindowOptions::default()
        },
    )
    .map_err(|e| io::Error::other(format!("could not open a window: {e}")))?;
    window.set_target_fps(60);

    while window.is_open() {
        let status = match log.last() {
            Some(a) => format!("last action: {a:?}   ·   Esc or Cancel to quit"),
            None => "Tab/arrows · Enter edits · Alt+letter jumps · Esc quits".to_string(),
        };

        let (composed, cursor) = compose(&state, screen, &status);
        let frame = rasterize(&composed, cursor, Theme::DEFAULT);
        window
            .update_with_buffer(&frame.to_argb(), frame.w, frame.h)
            .map_err(|e| io::Error::other(format!("could not present a frame: {e}")))?;

        let shift = window.is_key_down(Key::LeftShift) || window.is_key_down(Key::RightShift);
        let alt = window.is_key_down(Key::LeftAlt) || window.is_key_down(Key::RightAlt);
        for key in window.get_keys_pressed(KeyRepeat::Yes) {
            let Some(ev) = to_form_event(key, shift, alt) else {
                continue;
            };
            let outcome = state.handle(ev);
            log.extend(outcome.actions);
            if outcome.close {
                return report(&log);
            }
        }
    }

    report(&log)
}

fn report(log: &[Action]) -> io::Result<()> {
    if log.is_empty() {
        println!("Form closed with no actions emitted.");
    } else {
        println!("Actions the form emitted, in order:");
        for action in log {
            println!("  {action:?}");
        }
    }
    Ok(())
}
