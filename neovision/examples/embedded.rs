//! An `embedded-graphics` host for neovision.
//!
//! The third host, and the one that answers "could this run on a
//! microcontroller?". It draws through [`DrawTarget`], the trait every
//! `embedded-graphics` display driver implements, so the drawing code here is
//! the code you would ship against a real ILI9341 or ST7789 — only the
//! `DrawTarget` underneath changes.
//!
//! What that means concretely: [`draw_cells`] is generic over `DrawTarget` and
//! knows nothing about windows. Point it at an `ili9341::Ili9341` and it drives
//! a panel; point it at the [`SimDisplay`] below and it drives a window on your
//! desk. Nothing else in the file is load-bearing.
//!
//! ```console
//! cargo run --example embedded              # a 320x240 window
//! cargo run --example embedded -- --single  # one PPM frame, no display
//! ```
//!
//! # Why not `embedded-graphics-simulator`
//!
//! That crate is the usual way to run `embedded-graphics` on a desktop, and it
//! would be the obvious choice here — but it is SDL-backed, and SDL is a C
//! library needing system packages. Everything else in this repository builds
//! and runs with no system dependencies at all, and a demo is a poor reason to
//! give that up. `SimDisplay` is about forty lines and costs nothing.
//!
//! # Sizing
//!
//! 320x240 is the common small-TFT resolution. At 8x16 that is 40x15 cells,
//! which this form no longer fits; at 8x8 it is 40x30, which it does with room
//! to spare. Both faces ship in `neovision-core`, so a panel with rows to spare
//! can have the taller one.

use std::io::{self, Write};

use embedded_graphics::{
    pixelcolor::Rgb565,
    prelude::*,
    primitives::{PrimitiveStyle, Rectangle},
};
use minifb::{Key, KeyRepeat, Scale, Window, WindowOptions};

use neovision::neovision_core::font;
use neovision::{
    render_with_cursor, ButtonRole, Cell, CellBuffer, CellDraw, ChoiceOption, ClusterItem,
    ClusterStyle, CursorShape, Field, FieldKind, FormEvent, FormState, LayerStack,
    Point as CellPoint, Size as CellSize, TextCursor, Theme,
};

/// Panel geometry: a common small TFT, in cells of the 8x8 face.
const PX_W: u32 = 320;
const PX_H: u32 = 240;
const COLS: u16 = (PX_W / font::GLYPH_W as u32) as u16;
const ROWS: u16 = (PX_H / font::GLYPH_H_8 as u32) as u16;

// ---------------------------------------------------------------------------
// The part a real device would keep
// ---------------------------------------------------------------------------

/// The 16 VGA colours as `Rgb565`.
fn vga_rgb565(index: u8) -> Rgb565 {
    let (r, g, b) = match index & 0x0F {
        0x0 => (0x00, 0x00, 0x00),
        0x1 => (0x00, 0x00, 0xAA),
        0x2 => (0x00, 0xAA, 0x00),
        0x3 => (0x00, 0xAA, 0xAA),
        0x4 => (0xAA, 0x00, 0x00),
        0x5 => (0xAA, 0x00, 0xAA),
        0x6 => (0xAA, 0x55, 0x00),
        0x7 => (0xAA, 0xAA, 0xAA),
        0x8 => (0x55, 0x55, 0x55),
        0x9 => (0x55, 0x55, 0xFF),
        0xA => (0x55, 0xFF, 0x55),
        0xB => (0x55, 0xFF, 0xFF),
        0xC => (0xFF, 0x55, 0x55),
        0xD => (0xFF, 0x55, 0xFF),
        0xE => (0xFF, 0xFF, 0x55),
        _ => (0xFF, 0xFF, 0xFF),
    };
    Rgb565::new(r >> 3, g >> 2, b >> 3)
}

/// Blit a whole cell buffer to any `embedded-graphics` display.
///
/// Generic over the target on purpose: this is the entire integration, and it
/// compiles unchanged against a real panel driver. Pixels go out through
/// [`DrawTarget::draw_iter`], which every driver implements and many accelerate
/// — a driver that can stream a window does so without this code knowing.
///
/// The caret is drawn last, and only where the form asked for one.
pub fn draw_cells<D>(
    display: &mut D,
    buf: &CellBuffer,
    cursor: Option<TextCursor>,
    theme: Theme,
) -> Result<(), D::Error>
where
    D: DrawTarget<Color = Rgb565>,
{
    let gh = font::GLYPH_H_8;
    let gw = font::GLYPH_W;

    for row in 0..buf.rows {
        for col in 0..buf.cols {
            let cell = buf.get(col, row);
            let fg = vga_rgb565(cell.fg());
            let bg = vga_rgb565(cell.bg());
            let glyph = font::glyph_8x8(cell.ch);
            let ox = col as i32 * gw as i32;
            let oy = row as i32 * gh as i32;

            display.draw_iter(glyph.iter().enumerate().flat_map(|(y, &bits)| {
                (0..gw).map(move |x| {
                    // Bit 7 is the leftmost pixel.
                    let lit = (bits >> (gw - 1 - x)) & 1 != 0;
                    Pixel(
                        Point::new(ox + x as i32, oy + y as i32),
                        if lit { fg } else { bg },
                    )
                })
            }))?;
        }
    }

    if let Some(c) = cursor {
        draw_caret(display, buf, c, theme)?;
    }
    Ok(())
}

/// Draw the caret, which the toolkit reports but never draws itself.
fn draw_caret<D>(
    display: &mut D,
    buf: &CellBuffer,
    cursor: TextCursor,
    theme: Theme,
) -> Result<(), D::Error>
where
    D: DrawTarget<Color = Rgb565>,
{
    let gh = font::GLYPH_H_8 as i32;
    let gw = font::GLYPH_W as i32;
    let ox = cursor.col as i32 * gw;
    let oy = cursor.row as i32 * gh;
    let cell = buf.get(cursor.col, cursor.row);

    match cursor.shape {
        // Reverse video needs nothing from the theme: the cell's own two
        // colours simply trade places.
        CursorShape::Overtype => {
            let fg = vga_rgb565(cell.fg());
            let bg = vga_rgb565(cell.bg());
            let glyph = font::glyph_8x8(cell.ch);
            display.draw_iter(glyph.iter().enumerate().flat_map(|(y, &bits)| {
                (0..font::GLYPH_W).map(move |x| {
                    let lit = (bits >> (font::GLYPH_W - 1 - x)) & 1 != 0;
                    Pixel(
                        Point::new(ox + x as i32, oy + y as i32),
                        if lit { bg } else { fg },
                    )
                })
            }))
        }
        // An underline has no such rule, so it takes the theme's caret colour.
        CursorShape::Insert => Rectangle::new(
            Point::new(ox, oy + gh - 1),
            Size::new(font::GLYPH_W as u32, 1),
        )
        .into_styled(PrimitiveStyle::with_fill(vga_rgb565(theme.cursor)))
        .draw(display),
    }
}

// ---------------------------------------------------------------------------
// The part that only exists because this runs on a desk
// ---------------------------------------------------------------------------

/// A `DrawTarget` backed by a plain pixel buffer.
///
/// Stands in for a panel driver. Swapping it for a real one is the whole of
/// what porting this demo to hardware involves.
struct SimDisplay {
    px: Vec<u32>,
    w: u32,
    h: u32,
}

impl SimDisplay {
    fn new(w: u32, h: u32) -> Self {
        Self {
            px: vec![0; (w * h) as usize],
            w,
            h,
        }
    }
}

impl OriginDimensions for SimDisplay {
    fn size(&self) -> Size {
        Size::new(self.w, self.h)
    }
}

impl DrawTarget for SimDisplay {
    type Color = Rgb565;
    /// Writing to memory cannot fail; a real driver would put its bus error
    /// here, which is why `draw_cells` returns `Result` rather than `()`.
    type Error = core::convert::Infallible;

    fn draw_iter<I>(&mut self, pixels: I) -> Result<(), Self::Error>
    where
        I: IntoIterator<Item = Pixel<Self::Color>>,
    {
        for Pixel(point, colour) in pixels {
            if point.x < 0 || point.y < 0 {
                continue;
            }
            let (x, y) = (point.x as u32, point.y as u32);
            if x >= self.w || y >= self.h {
                continue;
            }
            // Rgb565 back to 8-bit channels for the window buffer.
            let r = (colour.r() as u32 * 255 + 15) / 31;
            let g = (colour.g() as u32 * 255 + 31) / 63;
            let b = (colour.b() as u32 * 255 + 15) / 31;
            self.px[(y * self.w + x) as usize] = (r << 16) | (g << 8) | b;
        }
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// Input
// ---------------------------------------------------------------------------

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

fn to_form_event(key: Key, shift: bool, alt: bool) -> Option<FormEvent> {
    if alt {
        if let Some(c) = letter_of(key) {
            return Some(FormEvent::Hotkey(c));
        }
    }
    if let Some(c) = letter_of(key) {
        return Some(FormEvent::Char(if shift {
            c.to_ascii_uppercase()
        } else {
            c
        }));
    }
    Some(match key {
        Key::Up => FormEvent::Up,
        Key::Down => FormEvent::Down,
        Key::Left => FormEvent::Left,
        Key::Right => FormEvent::Right,
        Key::Tab if shift => FormEvent::BackTab,
        Key::Tab => FormEvent::Tab,
        Key::Enter => FormEvent::Enter,
        Key::Escape => FormEvent::Escape,
        Key::Space => FormEvent::Char(' '),
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

// ---------------------------------------------------------------------------
// The form
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Eq)]
enum Action {
    SetMode(&'static str),
    SetBacklight(u32),
    SetName(String),
    Invert(bool),
    Save,
}

fn demo_form() -> FormState<Action> {
    FormState::new(
        " Panel Setup ",
        vec![
            Field {
                label: "Driver",
                kind: FieldKind::Cluster {
                    style: ClusterStyle::Radio,
                    items: vec![
                        ClusterItem {
                            label: "ILI9341".to_string(),
                            on: true,
                            on_action: Action::SetMode("ILI9341"),
                            off_action: None,
                        },
                        ClusterItem {
                            label: "ST7789".to_string(),
                            on: false,
                            on_action: Action::SetMode("ST7789"),
                            off_action: None,
                        },
                    ],
                    cursor: 0,
                },
                restore: vec![Action::SetMode("ILI9341")],
            },
            Field {
                label: "Options",
                kind: FieldKind::Cluster {
                    style: ClusterStyle::Check,
                    items: vec![ClusterItem {
                        label: "Invert".to_string(),
                        on: false,
                        on_action: Action::Invert(true),
                        off_action: Some(Action::Invert(false)),
                    }],
                    cursor: 0,
                },
                restore: vec![Action::Invert(false)],
            },
            Field {
                label: "Rotation",
                kind: FieldKind::Choice {
                    options: ["0°", "90°", "180°", "270°"]
                        .iter()
                        .map(|d| ChoiceOption {
                            label: d.to_string(),
                            action: Action::SetMode("rotate"),
                        })
                        .collect(),
                    selected: Some(0),
                },
                restore: vec![],
            },
            Field {
                label: "Name",
                kind: FieldKind::Text {
                    buffer: "panel0".to_string(),
                    cursor: "panel0".len(),
                    selected: true,
                    overtype: false,
                    max_len: 16,
                    commit: |s| Action::SetName(s.to_string()),
                },
                restore: vec![Action::SetName("panel0".to_string())],
            },
            Field {
                label: "Backlight",
                kind: FieldKind::Number {
                    value: 80,
                    buffer: "80".to_string(),
                    cursor: 2,
                    selected: true,
                    overtype: false,
                    min: 0,
                    max: 100,
                    unit: "%",
                    commit: Action::SetBacklight,
                },
                restore: vec![Action::SetBacklight(80)],
            },
            Field::button("~A~pply", ButtonRole::Stay, Some(Action::Save)),
            Field::ok(),
            Field::cancel(),
        ],
    )
    // Left on the library default, `EnterReach::OperateOnly`: Enter operates
    // the focused control and never closes the form. `with_enter_reach` opts
    // into the dialog readings, but a demo that quietly took one would
    // misrepresent what the toolkit does out of the box.
}

fn desktop(size: CellSize, status: &str) -> CellBuffer {
    let mut buf = CellBuffer::new(size.w, size.h);
    buf.fill(Cell::new(0xB0, 0x08, 0x01, false));
    buf.fill_row(0, Cell::new(b' ', 0x0, 0x3, false));
    buf.write_str(CellPoint::new(1, 0), "neovision \u{00B7} 320x240 TFT", 0x30);
    let footer = size.h.saturating_sub(1);
    buf.fill_row(footer, Cell::new(b' ', 0x0, 0x3, false));
    buf.write_str(CellPoint::new(1, footer), status, 0x30);
    buf
}

fn compose(
    state: &FormState<Action>,
    screen: CellSize,
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

// 40 columns is not many; this has to fit inside them.
const STATUS: &str = "Tab \u{00B7} Space picks \u{00B7} Alt+key \u{00B7} Esc";

fn main() -> io::Result<()> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.iter().any(|a| a == "--single") {
        return single_frame(&args);
    }
    interactive()
}

fn render_to_buffer(state: &FormState<Action>) -> SimDisplay {
    let screen = CellSize::new(COLS, ROWS);
    let (composed, cursor) = compose(state, screen, STATUS);
    let mut display = SimDisplay::new(PX_W, PX_H);
    // Infallible, so the error cannot happen — but the call still returns one,
    // which is the point: the same line against a real panel can fail.
    let Ok(()) = draw_cells(&mut display, &composed, cursor, Theme::DEFAULT);
    display
}

fn single_frame(args: &[String]) -> io::Result<()> {
    let path = args
        .iter()
        .position(|a| a == "--single")
        .and_then(|i| args.get(i + 1))
        .filter(|a| !a.starts_with("--"))
        .cloned()
        .unwrap_or_else(|| "embedded.ppm".to_string());

    let display = render_to_buffer(&demo_form());
    let mut out = io::BufWriter::new(std::fs::File::create(&path)?);
    write!(out, "P6\n{} {}\n255\n", display.w, display.h)?;
    let mut rgb = Vec::with_capacity(display.px.len() * 3);
    for &p in &display.px {
        rgb.push((p >> 16) as u8);
        rgb.push((p >> 8) as u8);
        rgb.push(p as u8);
    }
    out.write_all(&rgb)?;
    out.flush()?;
    println!("wrote {} ({}x{} px, {COLS}x{ROWS} cells)", path, PX_W, PX_H);
    Ok(())
}

fn interactive() -> io::Result<()> {
    let mut state = demo_form();
    let mut window = Window::new(
        "neovision - embedded-graphics (320x240)",
        PX_W as usize,
        PX_H as usize,
        WindowOptions {
            scale: Scale::X2,
            ..WindowOptions::default()
        },
    )
    .map_err(|e| io::Error::other(format!("could not open a window: {e}")))?;
    window.set_target_fps(60);

    let mut log: Vec<Action> = Vec::new();
    while window.is_open() {
        let display = render_to_buffer(&state);
        window
            .update_with_buffer(&display.px, PX_W as usize, PX_H as usize)
            .map_err(|e| io::Error::other(format!("could not present: {e}")))?;

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
        for a in log {
            println!("  {a:?}");
        }
    }
    Ok(())
}
