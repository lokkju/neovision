//! A terminal host for neovision.
//!
//! neovision renders to a character-cell grid and stops there. Turning that
//! grid into something a human can see is the host's job, and a host is
//! smaller than it sounds — it owes the toolkit exactly three things:
//!
//! 1. **Glyphs.** A cell holds a CP437 byte; [`cp437::to_char`] turns it into
//!    something a terminal can print.
//! 2. **Colour.** A cell holds a VGA attribute byte; [`vga_color`] splits it
//!    into a foreground and background the terminal understands.
//! 3. **Input.** The toolkit speaks [`FormEvent`], never a terminal key code,
//!    so [`to_form_event`] does the translation.
//!
//! Everything else here is scaffolding. Run it with:
//!
//! ```console
//! cargo run --example terminal
//! ```
//!
//! Tab/arrows move, Enter opens a choice or commits a number, Esc backs out,
//! and the status bar shows the actions the form emits as you go.

use std::io::{self, Write};

use crossterm::{
    cursor::{Hide, MoveTo, SetCursorStyle, Show},
    event::{self, Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers},
    execute, queue,
    style::{Attribute, Color, Print, SetAttribute, SetBackgroundColor, SetForegroundColor},
    terminal::{
        disable_raw_mode, enable_raw_mode, size as terminal_size, EnterAlternateScreen,
        LeaveAlternateScreen,
    },
};

use neovision::neovision_core::cp437;
use neovision::{
    render_with_cursor, Cell, CellBuffer, CellDraw, ChoiceOption, ClusterItem, ClusterStyle,
    CursorShape, EnterReach, Field, FieldKind, FormEvent, FormState, LayerStack, Point, Size,
    TextCursor, Theme,
};

/// What this demo's form can ask for.
///
/// neovision is generic over this type and never inspects it — the form hands
/// values back and the host decides what they mean. That is the whole of the
/// contract, and it is why the toolkit carries no dependency on any app.
#[derive(Debug, Clone, PartialEq, Eq)]
#[allow(clippy::enum_variant_names)] // They really are all setters.
enum Action {
    SetTheme(&'static str),
    SetScale(&'static str),
    SetSound(bool),
    SetDelay(u32),
    SetName(String),
}

// ---------------------------------------------------------------------------
// 1. Colour: VGA attribute byte -> terminal colour
// ---------------------------------------------------------------------------

/// The 16-entry VGA palette, in its canonical order.
///
/// A background nibble only carries three bits (bit 7 is blink), so
/// backgrounds land in the first eight entries.
fn vga_color(index: u8) -> Color {
    match index & 0x0F {
        0x0 => Color::Black,
        0x1 => Color::DarkBlue,
        0x2 => Color::DarkGreen,
        0x3 => Color::DarkCyan,
        0x4 => Color::DarkRed,
        0x5 => Color::DarkMagenta,
        0x6 => Color::DarkYellow,
        0x7 => Color::Grey,
        0x8 => Color::DarkGrey,
        0x9 => Color::Blue,
        0xA => Color::Green,
        0xB => Color::Cyan,
        0xC => Color::Red,
        0xD => Color::Magenta,
        0xE => Color::Yellow,
        _ => Color::White,
    }
}

/// The same 16 colours as RGB, for the one place a `Color` will not do.
///
/// A terminal draws its caret in a colour it was configured with, not in one
/// derived from the cell underneath, so an underline caret on the focused row's
/// light-grey bar is invisible unless the host says otherwise. OSC 12 is how
/// you say otherwise, and it takes RGB.
fn vga_rgb(index: u8) -> (u8, u8, u8) {
    match index & 0x0F {
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
    }
}

// ---------------------------------------------------------------------------
// 2. Input: terminal key -> FormEvent
// ---------------------------------------------------------------------------

/// Translate a terminal key into the toolkit's own input vocabulary.
///
/// Returning `None` means "the form has no opinion about this key" — the host
/// keeps such keys for itself.
fn to_form_event(key: KeyEvent) -> Option<FormEvent> {
    // Alt+letter is this host's accelerator. Emitting `Hotkey` is entirely
    // optional — a host with no Alt simply never sends it, and the form works
    // the same minus accelerators.
    if key.modifiers.contains(KeyModifiers::ALT) {
        if let KeyCode::Char(c) = key.code {
            return Some(FormEvent::Hotkey(c));
        }
    }
    Some(match key.code {
        KeyCode::Up => FormEvent::Up,
        KeyCode::Down => FormEvent::Down,
        KeyCode::Left => FormEvent::Left,
        KeyCode::Right => FormEvent::Right,
        KeyCode::Tab => FormEvent::Tab,
        KeyCode::BackTab => FormEvent::BackTab,
        KeyCode::Enter => FormEvent::Enter,
        KeyCode::Esc => FormEvent::Escape,
        KeyCode::Backspace => FormEvent::Backspace,
        KeyCode::Home => FormEvent::Home,
        KeyCode::End => FormEvent::End,
        KeyCode::Delete => FormEvent::Delete,
        KeyCode::Insert => FormEvent::Insert,
        KeyCode::Char(c) => FormEvent::Char(c),
        _ => return None,
    })
}

// ---------------------------------------------------------------------------
// 3. Glyphs: cell buffer -> terminal
// ---------------------------------------------------------------------------

/// Paint a whole cell buffer, then place the caret.
///
/// Attribute changes are emitted only when they actually change, so a screen
/// of mostly-uniform colour costs a handful of escape sequences rather than
/// one per cell.
fn paint(
    out: &mut impl Write,
    buf: &CellBuffer,
    cursor: Option<TextCursor>,
    theme: Theme,
) -> io::Result<()> {
    queue!(out, Hide)?;

    let mut last_attr: Option<u8> = None;
    for row in 0..buf.rows {
        queue!(out, MoveTo(0, row))?;
        for col in 0..buf.cols {
            let cell = buf.get(col, row);
            if last_attr != Some(cell.attr) {
                queue!(
                    out,
                    SetForegroundColor(vga_color(cell.fg())),
                    SetBackgroundColor(vga_color(cell.bg())),
                    SetAttribute(if cell.blink() {
                        Attribute::SlowBlink
                    } else {
                        Attribute::NoBlink
                    })
                )?;
                last_attr = Some(cell.attr);
            }
            queue!(out, Print(cp437::to_char(cell.ch)))?;
        }
    }

    match cursor {
        Some(c) => {
            // Tell the terminal what colour the caret is. Without this it uses
            // whatever the user configured — usually something pale, which
            // disappears against the focused row's light-grey bar. `theme.cursor`
            // is chosen to contrast against exactly that background.
            let (r, g, b) = vga_rgb(theme.cursor);
            write!(out, "\x1b]12;#{r:02X}{g:02X}{b:02X}\x07")?;
            queue!(
                out,
                MoveTo(c.col, c.row),
                match c.shape {
                    CursorShape::Insert => SetCursorStyle::SteadyUnderScore,
                    CursorShape::Overtype => SetCursorStyle::SteadyBlock,
                },
                Show
            )?
        }
        None => queue!(out, Hide)?,
    }

    out.flush()
}

// ---------------------------------------------------------------------------
// The form itself
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
        " Display Settings ",
        vec![
            choice(
                "Theme",
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
                            on_action: Action::SetTheme("CGA"),
                            off_action: None,
                        },
                        ClusterItem {
                            label: "EGA".to_string(),
                            on: false,
                            on_action: Action::SetTheme("EGA"),
                            off_action: None,
                        },
                        ClusterItem {
                            label: "VGA".to_string(),
                            on: false,
                            on_action: Action::SetTheme("VGA"),
                            off_action: None,
                        },
                    ],
                    cursor: 0,
                },
                restore: vec![Action::SetTheme("CGA")],
            },
            Field {
                label: "Options",
                kind: FieldKind::Cluster {
                    style: ClusterStyle::Check,
                    items: vec![
                        ClusterItem {
                            label: "Scanlines".to_string(),
                            on: true,
                            on_action: Action::SetSound(true),
                            off_action: Some(Action::SetSound(false)),
                        },
                        ClusterItem {
                            label: "Blink".to_string(),
                            on: false,
                            on_action: Action::SetSound(true),
                            off_action: Some(Action::SetSound(false)),
                        },
                    ],
                    cursor: 0,
                },
                restore: vec![],
            },
            Field {
                label: "Sound",
                kind: FieldKind::Toggle {
                    on: true,
                    on_action: Action::SetSound(true),
                    off_action: Action::SetSound(false),
                },
                restore: vec![Action::SetSound(true)],
            },
            Field {
                label: "Frame delay",
                kind: FieldKind::Number {
                    value: 250,
                    buffer: "250".to_string(),
                    cursor: 3,
                    selected: true,
                    overtype: false,
                    min: 0,
                    max: 9999,
                    unit: "ms",
                    commit: Action::SetDelay,
                },
                restore: vec![Action::SetDelay(250)],
            },
            Field {
                label: "Renderer",
                kind: FieldKind::ReadOnly("terminal (cp437)".to_string()),
                restore: vec![],
            },
            Field::ok(),
            Field::cancel(),
        ],
    )
    .with_enter_reach(EnterReach::AcceptWhenIdle)
}

/// The desktop the dialog floats over, plus the two chrome bars.
///
/// Drawn straight onto a [`CellBuffer`] with [`CellDraw`], which is the same
/// vocabulary the widget renderer uses — the toolkit has no privileged access
/// a host lacks.
fn desktop(size: Size, status: &str) -> CellBuffer {
    let mut buf = CellBuffer::new(size.w, size.h);
    // 0xB0 is the light shade block: the classic text-mode desktop fill.
    buf.fill(Cell::new(0xB0, 0x08, 0x01, false));

    buf.fill_row(0, Cell::new(b' ', 0x0, 0x3, false));
    // CP437 has no em dash, so the chrome sticks to glyphs it does have.
    buf.write_str(Point::new(2, 0), "neovision · terminal host", 0x30);

    let footer = size.h.saturating_sub(1);
    buf.fill_row(footer, Cell::new(b' ', 0x0, 0x3, false));
    buf.write_str(Point::new(2, footer), status, 0x30);

    buf
}

/// Build the frame the host would paint: desktop, dialog layers, caret.
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

/// Render one frame to stdout as plain text and exit.
///
/// No raw mode and no terminal required, so `--dump` works over a pipe and in
/// CI — which makes the rendering checkable without a human driving it.
/// Parse one `--keys` token into an event, so a headless run can reach a state
/// a human would have to tab into.
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

fn dump(args: &[String]) -> io::Result<()> {
    let mut state = demo_form();
    if let Some(i) = args.iter().position(|a| a == "--keys") {
        let keys = args.get(i + 1).cloned().unwrap_or_default();
        for token in keys.split(',').filter(|t| !t.is_empty()) {
            let Some(ev) = parse_key(token) else {
                return Err(io::Error::other(format!("unknown key token: {token}")));
            };
            state.handle(ev);
        }
    }

    let screen = Size::new(80, 25);
    let (composed, cursor) = compose(
        &state,
        screen,
        "Tab/arrows · Enter edits · Alt+letter jumps · Esc quits",
    );

    for row in 0..composed.rows {
        let line: String = (0..composed.cols)
            .map(|col| cp437::to_char(composed.get(col, row).ch))
            .collect();
        println!("{}", line.trim_end());
    }
    match cursor {
        Some(c) => println!("\ncaret: col {} row {} ({:?})", c.col, c.row, c.shape),
        None => println!("\ncaret: none"),
    }
    Ok(())
}

fn main() -> io::Result<()> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.iter().any(|a| a == "--dump") {
        return dump(&args);
    }

    let mut stdout = io::stdout();
    enable_raw_mode()?;
    execute!(stdout, EnterAlternateScreen, Hide)?;

    let result = run(&mut stdout);

    // OSC 112 hands the caret colour back to whatever the user had configured.
    let _ = write!(stdout, "\x1b]112\x07");
    execute!(
        stdout,
        Show,
        SetCursorStyle::DefaultUserShape,
        LeaveAlternateScreen
    )?;
    disable_raw_mode()?;

    // Report outside the alternate screen so the log survives the demo.
    match &result {
        Ok(log) if log.is_empty() => println!("Form closed with no actions emitted."),
        Ok(log) => {
            println!("Actions the form emitted, in order:");
            for action in log {
                println!("  {action:?}");
            }
        }
        Err(e) => eprintln!("terminal host failed: {e}"),
    }
    result.map(|_| ())
}

fn run(stdout: &mut impl Write) -> io::Result<Vec<Action>> {
    let mut state = demo_form();
    let mut log: Vec<Action> = Vec::new();
    let (cols, rows) = terminal_size()?;
    let mut screen = Size::new(cols.max(60), rows.max(18));

    loop {
        let status = match log.last() {
            Some(a) => format!("last action: {a:?}   ·   Esc or Cancel to quit"),
            None => "Tab/arrows · Enter edits · Alt+letter jumps · Esc quits".to_string(),
        };

        let (composed, cursor) = compose(&state, screen, &status);
        paint(stdout, &composed, cursor, Theme::DEFAULT)?;

        match event::read()? {
            Event::Resize(w, h) => {
                screen = Size::new(w.max(60), h.max(18));
            }
            Event::Key(key) if key.kind == KeyEventKind::Press => {
                // Ctrl+C is the host's, not the form's.
                if key.code == KeyCode::Char('c') && key.modifiers.contains(KeyModifiers::CONTROL) {
                    return Ok(log);
                }
                let Some(ev) = to_form_event(key) else {
                    continue;
                };
                let outcome = state.handle(ev);
                log.extend(outcome.actions);
                if outcome.close {
                    return Ok(log);
                }
            }
            _ => {}
        }
    }
}
