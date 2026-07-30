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

```
░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░
░░░░░░░░░░░░░░░░░░░░╔════════  Display Settings  ═════════╗░░░░░░░░░░░░░░░░░░░░░
░░░░░░░░░░░░░░░░░░░░║ Theme         [Classic             ]║░░░░░░░░░░░░░░░░░░░░░
░░░░░░░░░░░░░░░░░░░░║ Scale         [2x                  ]║░░░░░░░░░░░░░░░░░░░░░
░░░░░░░░░░░░░░░░░░░░║ Sound         [Yes                 ]║░░░░░░░░░░░░░░░░░░░░░
░░░░░░░░░░░░░░░░░░░░║ Frame delay   [250  ]ms (0-9999)    ║░░░░░░░░░░░░░░░░░░░░░
░░░░░░░░░░░░░░░░░░░░║ Renderer      terminal (cp437)      ║░░░░░░░░░░░░░░░░░░░░░
░░░░░░░░░░░░░░░░░░░░╠═════════════════════════════════════╣░░░░░░░░░░░░░░░░░░░░░
░░░░░░░░░░░░░░░░░░░░║       [  OK  ]    [ Cancel ]        ║░░░░░░░░░░░░░░░░░░░░░
░░░░░░░░░░░░░░░░░░░░╚═════════════════════════════════════╝░░░░░░░░░░░░░░░░░░░░░
░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░
```

Try it:

```console
cargo run --example terminal            # drive it in your terminal
cargo run --example terminal -- --dump  # render one frame as text, no TTY

cargo run --example framebuffer --features font              # a pixel window
cargo run --example framebuffer --features font -- --single  # write one PPM
```

## The crates

| Crate | What it is |
|---|---|
| [`neovision`](https://docs.rs/neovision) | The widget toolkit: forms, fields, the CUA renderer. Re-exports `neovision-core`. |
| [`neovision-core`](https://docs.rs/neovision-core) | The substrate: cells, layers, the compositor, CP437, geometry. Zero dependencies. |

`neovision-core` also bundles the IBM VGA 8x16 face behind a `font` feature,
off by default. Only hosts that rasterize pixels need it, and 4 KiB of glyph
data is not free on the bare-metal targets `no_std` is there to reach.

`cargo add neovision` gets you both — the widgets and the primitives they draw
on. Reach for `neovision-core` alone only if you want the cell grid without any
opinion about widgets.

## Writing a host

A host owes the toolkit three things, and nothing else. There are two complete
ones to read: `examples/terminal.rs` hands cells to a terminal, and
`examples/framebuffer.rs` rasterizes every pixel itself the way a canvas, an
embedded display, or a real VESA mode would. Both are mostly comments.

Each has a headless mode that renders without a display — `--dump` for the
terminal host, `--single` for the framebuffer one — so what they draw is
checked in CI rather than only by eye.

**Glyphs.** A cell's `ch` is a CP437 byte. `cp437::to_char` turns it into a
`char` for a terminal; a pixel host calls `font::glyph` and walks the bits.

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

## Generic over your actions

`FormState<A>` carries values of *your* action type. It stores them, hands them
back when the user picks something, and never looks inside. That is what keeps
the toolkit free of any dependency on the application using it — and what lets
a form describe what the user asked for without knowing how to do it.

Cancel is built on the same idea: each field carries the actions that restore
its original value, and cancelling replays those for the fields the user
actually touched.

## Status

Pre-1.0 and honest about it. The form widgets, the renderer, and the compositor
are covered by 153 tests. The API will still move.

Deliberately **not** implemented yet: Turbo Vision's `TGroup` / `TApplication`
layer — a view tree, a desktop, stacked modal dialogs, z-ordered windows with
focus chains. neovision models one modal overlay at a time. That layer will get
built when something real needs it, not before.

## License

MIT. See [LICENSE](LICENSE).
