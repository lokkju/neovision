# neovision

[![CI](https://github.com/lokkju/neovision/actions/workflows/ci.yml/badge.svg)](https://github.com/lokkju/neovision/actions/workflows/ci.yml)
[![crates.io](https://img.shields.io/crates/v/neovision.svg)](https://crates.io/crates/neovision)
[![docs.rs](https://docs.rs/neovision/badge.svg)](https://docs.rs/neovision)
[![MIT](https://img.shields.io/badge/license-MIT-blue.svg)](LICENSE)

A `no_std` toolkit for building DOS / Turbo-Vision-style (CUA) text UIs on a
character-cell grid.

Unlike ratatui or cursive, neovision is **not terminal-bound**. It renders to a
buffer of `Cell`s — a CP437 character byte plus a VGA attribute byte, the exact
layout of text-mode video memory — and stops there. What turns that buffer into
pixels is entirely up to the host: a terminal, a framebuffer, a wasm canvas, or
real VGA memory at `0xB8000`.

![neovision, driven in the framebuffer host](https://raw.githubusercontent.com/lokkju/neovision/main/docs/demo.gif)

Above is the `framebuffer` example, rasterized straight from the cell buffer at
640x400 with the IBM VGA 8x16 face. Its 16 colours are the GIF's whole palette —
nothing is quantized, because a VGA attribute nibble already *is* a palette
index.

The same form through the `terminal` host, as `--dump` prints it:

```text
░░░░░░░░░░░░░░░░░░░░╔════════  Display Settings  ═════════╗░░░░░░░░░░░░░░░░░░░░░
░░░░░░░░░░░░░░░░░░░░║ Theme         [Classic             ]║░░░░░░░░░░░░░░░░░░░░░
░░░░░░░░░░░░░░░░░░░░║ Scale         [2x                  ]║░░░░░░░░░░░░░░░░░░░░░
░░░░░░░░░░░░░░░░░░░░║ Profile       [default             ]║░░░░░░░░░░░░░░░░░░░░░
░░░░░░░░░░░░░░░░░░░░║ Video         (•) CGA               ║░░░░░░░░░░░░░░░░░░░░░
░░░░░░░░░░░░░░░░░░░░║               ( ) EGA               ║░░░░░░░░░░░░░░░░░░░░░
░░░░░░░░░░░░░░░░░░░░║               ( ) VGA               ║░░░░░░░░░░░░░░░░░░░░░
░░░░░░░░░░░░░░░░░░░░║ Options       [X] Scanlines         ║░░░░░░░░░░░░░░░░░░░░░
░░░░░░░░░░░░░░░░░░░░║               [ ] Blink             ║░░░░░░░░░░░░░░░░░░░░░
░░░░░░░░░░░░░░░░░░░░║ Sound         [Yes                 ]║░░░░░░░░░░░░░░░░░░░░░
░░░░░░░░░░░░░░░░░░░░║ Frame delay   [250  ]ms (0-9999)    ║░░░░░░░░░░░░░░░░░░░░░
░░░░░░░░░░░░░░░░░░░░║ Renderer      terminal (cp437)      ║░░░░░░░░░░░░░░░░░░░░░
░░░░░░░░░░░░░░░░░░░░╠═════════════════════════════════════╣░░░░░░░░░░░░░░░░░░░░░
░░░░░░░░░░░░░░░░░░░░║       «  OK  »    [ Cancel ]        ║░░░░░░░░░░░░░░░░░░░░░
░░░░░░░░░░░░░░░░░░░░╚═════════════════════════════════════╝░░░░░░░░░░░░░░░░░░░░░
░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░
░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░
```

Try it:

```console
cargo run --example terminal            # drive it in your terminal
cargo run --example terminal -- --dump  # render one frame as text, no TTY

cargo run --example framebuffer              # a pixel window
cargo run --example framebuffer -- --single  # write one PPM

cargo run --example embedded                 # a simulated 320x240 TFT
cargo run --example embedded -- --single     # write one PPM
```

## The crates

| Crate | What it is |
|---|---|
| [`neovision`](https://docs.rs/neovision) | The widget toolkit: forms, fields, the CUA renderer. Re-exports `neovision-core`. |
| [`neovision-core`](https://docs.rs/neovision-core) | The substrate: cells, layers, the compositor, CP437, geometry. Zero dependencies. |

`neovision` bundles the IBM VGA faces — 8x16 and 8x8 — by default, so a pixel host can draw
without hunting for a font. It comes from `neovision-core`'s `font` feature,
which is off *there* by default — the substrate stays minimal for anyone
counting bytes. A build that cannot spare the 4 KiB takes
`neovision = { version = "1", default-features = false }`.

`cargo add neovision` gets you both — the widgets and the primitives they draw
on. Reach for `neovision-core` alone only if you want the cell grid without any
opinion about widgets.

## Building a form

A form is a title and a list of fields, generic over your own action type.

```rust
use neovision::{
    ChoiceOption, ClusterItem, ClusterStyle, EnterReach, Field, FieldKind, FormState,
};

#[derive(Clone)]
enum Action {
    SetTheme(&'static str),
    SetName(String),
    SetDelay(u32),
}

fn settings() -> FormState<Action> {
    FormState::new(
        " Settings ",
        vec![
            // A dropdown. Enter opens it; the popup scrolls if it has to.
            Field {
                label: "Theme",
                kind: FieldKind::Choice {
                    options: vec![
                        ChoiceOption { label: "Classic".into(), action: Action::SetTheme("classic") },
                        ChoiceOption { label: "Amber".into(),   action: Action::SetTheme("amber") },
                    ],
                    selected: Some(0),
                },
                // What Cancel replays if the user changed this field.
                restore: vec![Action::SetTheme("classic")],
            },
            // Free text, with the whole value selected on entry.
            Field {
                label: "Name",
                kind: FieldKind::Text {
                    buffer: "default".into(),
                    cursor: 7,
                    selected: true,
                    overtype: false,
                    max_len: 32,
                    commit: |s| Action::SetName(s.to_string()),
                },
                restore: vec![Action::SetName("default".into())],
            },
            // A radio cluster: arrows move the caret, Space chooses.
            Field {
                label: "Video",
                kind: FieldKind::Cluster {
                    style: ClusterStyle::Radio,
                    items: vec![
                        ClusterItem { label: "CGA".into(), on: true,  on_action: Action::SetTheme("cga"), off_action: None },
                        ClusterItem { label: "EGA".into(), on: false, on_action: Action::SetTheme("ega"), off_action: None },
                    ],
                    cursor: 0,
                },
                restore: vec![Action::SetTheme("cga")],
            },
            // Bounded integer entry.
            Field {
                label: "Delay",
                kind: FieldKind::Number {
                    value: 250,
                    buffer: "250".into(),
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
            Field::ok(),     // default button: `«  OK  »`, Alt+O
            Field::cancel(), // Alt+C, and what Escape does
        ],
    )
    // Enter finishes the form. The library default is `OperateOnly`, where
    // Enter only ever operates the focused control.
    .with_enter_reach(EnterReach::AcceptWhenIdle)
}
```

The remaining kinds are `Toggle`, `ReadOnly`, and `Cluster` with
`ClusterStyle::Check`. `Theme` varies the attributes and `Layout` the column
widths; `render_themed` takes both.

## Writing a host

A host owes the toolkit three things, and nothing else. There are three
complete ones to read:

| example | what it drives |
|---|---|
| `terminal.rs` | hands cells to a terminal via crossterm |
| `framebuffer.rs` | rasterizes every pixel itself, the way a canvas or a real VESA mode would |
| `embedded.rs` | draws through `embedded-graphics`' `DrawTarget`, so the same code runs against a real ILI9341 or ST7789 |

All three are mostly comments.

Each has a headless mode that renders without a display — `--dump` for the
terminal host, `--single` for the other two — so what they draw is checked in
CI rather than only by eye. All three also take `--keys tab,tab,down,space`, to
reach a state without driving it by hand.

**Glyphs.** A cell's `ch` is a CP437 byte. `cp437::to_char` turns it into a
`char` for a terminal; a pixel host calls `font::glyph` (8x16) or
`font::glyph_8x8` and walks the bits.

**Colour.** A cell's `attr` is a VGA attribute byte: `fg()` gives 16 foreground
colours, `bg()` gives 8 background colours, `blink()` gives bit 7.

**Input.** The toolkit speaks `FormEvent`, never a terminal key code or a
scancode. The host translates.

```rust
use neovision::{render_with_cursor, CellBuffer, FormEvent, FormState, LayerStack, Size};

fn frame<A: Clone>(state: &mut FormState<A>, base: &CellBuffer, key: FormEvent) -> Vec<A> {
    let screen = Size::new(80, 25);

    // Render: state in, layers out. Pure — nothing is mutated.
    let (layers, _cursor) = render_with_cursor(state, screen);

    let mut stack = LayerStack::new();
    for layer in layers {
        stack.push(layer);
    }
    let mut composed = CellBuffer::new(screen.w, screen.h);
    stack.composite(base, &mut composed);
    // ...blit `composed` however your host draws.

    // Input: your key, already translated into a FormEvent.
    // The actions handed back are yours; neovision never inspects them.
    state.handle(key).actions
}
```

## Why it behaves as it does

Keyboard behaviour in a text-mode form is a pile of small decisions, several of
which depart from a dominant convention on purpose.
[`docs/ux-decisions.md`](docs/ux-decisions.md) records each one with its
reasoning, the precedent it follows or departs from, and what it costs.

## Generic over your actions

`FormState<A>` carries values of *your* action type. It stores them, hands them
back when the user picks something, and never looks inside. That is what keeps
the toolkit free of any dependency on the application using it — and what lets
a form describe what the user asked for without knowing how to do it.

Cancel is built on the same idea: each field carries the actions that restore
its original value, and cancelling replays those for the fields the user
actually touched.

## Status

1.0, and the API is what it commits to. Over 230 tests cover the widgets, the
renderer and the compositor; CI additionally checks clippy and rustdoc at
`-D warnings`, the MSRV of **1.76** against that toolchain, a bare-metal
`thumbv7em-none-eabihf` build, and a real rendered frame from each of the three
hosts.

Deliberately **not** implemented, and each waiting on a real consumer rather
than a schedule:

- **`TGroup` / `TApplication`** — a view tree, a desktop, stacked modal dialogs,
  z-ordered windows with focus chains. neovision models one modal overlay at a
  time. Placing two forms side by side does *not* need this; it needs a
  placement parameter, which is planned.
- **Validators** — Turbo Vision's range, picture, filter and lookup family.
  `Number` clamps to `min`/`max` and nothing else validates.
- **`TMemo`** — multi-line text. The edit state machine is single-line
  throughout, so this is a new field kind rather than a flag on `Text`.
- **`THistory`** — a dropdown of previously entered values on an input line.

## License

MIT. See [LICENSE](LICENSE).
