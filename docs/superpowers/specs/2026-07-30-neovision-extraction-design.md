# neovision — design and extraction record

> **Status:** as-built. This records what neovision is, how it is structured,
> and the decisions taken when it was extracted into its own repository on
> 2026-07-30. Items still open are marked as such.

## What it is

**neovision** is a `no_std` + `alloc`, **renderer-agnostic toolkit for building
DOS / Turbo-Vision-style (CUA) text UIs on a character-cell grid.**

Unlike ratatui or cursive it is **not terminal-bound**. It emits a buffer of
`Cell`s — a CP437 character byte plus a VGA attribute byte, the exact layout of
text-mode video memory — plus composited layers, and stops there. What turns
that buffer into something visible is entirely the host's business: a pixel
framebuffer, a wasm canvas, real VGA memory at `0xB8000`, or a terminal.

The identity is *retro / CUA TUI*. "Cell" only names the substrate.

The name was chosen for its Turbo Vision lineage — the CUA form/dialog
tradition it descends from.

## Crate structure

```
neovision-core   Substrate. Cell / CellBuffer, Layer / LayerStack + compositor,
                 TextCursor / CursorShape, BoxChars, CellDraw, the CP437 table,
                 geom (Point/Rect/Size). Zero dependencies.
        ▲
neovision        The CUA widget toolkit. FormState / Field / FieldKind
                 (Choice/Number/Toggle/ReadOnly/Button), the entry-field editing
                 state machine, the CUA renderer (render / render_with_cursor →
                 Layers + TextCursor), FormTheme, FormEvent / FormOutcome.
                 Re-exports neovision-core, so `cargo add neovision` yields
                 widgets + primitives.

examples/        Hosts. Living documentation, never published as a library.
```

### Why there is no third "controller" crate

An earlier sketch had a `neovision-cua` crate holding a modal controller. That
was over-engineering, and it was dropped. Separating what is genuinely reusable
from what is application policy:

- **Reusable (thin):** hold the current overlay, route `FormEvent`s to it,
  surface its actions, open and close it. Small enough to be a module if it is
  ever wanted at all — it is a convenience over `FormState::handle`, not a
  separate concern.
- **Application policy (not reusable):** *which* overlays exist, how they are
  triggered, the hub UX, the host key → `FormEvent` mapping, and how a given
  runtime's UI seam is implemented. Every consumer writes its own; shipping one
  application's opinions as a library would ship opinions adopters replace.
  This belongs in an example.

The generic form-driver helper was deliberately **not** written at extraction
time either. The only generic nugget available was a five-line
`FormState::handle` loop, which would barely exceed `FormState` itself. It gets
written when a real consumer needs it.

## The dependency-purity rule

`neovision` depends on **only** `neovision-core`. Neither crate may depend on
any consuming application's types — no host `Key` enum, no host UI-controller
trait, no application `Action` type.

The mechanism that enforces this is the generic parameter. `FormState<A>`
carries values of the caller's action type `A`: it stores them, hands them back
when the user picks something, and **never inspects them**. A form can therefore
describe what the user asked for without knowing how to do it.

The model's own tests declare a local dummy action type rather than importing
one. If the model ever grows a dependency on some application's action enum,
those tests stop compiling — which is the point.

Input is subject to the same rule. The toolkit speaks `FormEvent`, its own key
abstraction, never a terminal key code or a scancode. Hosts translate.

## What a host owes the toolkit

Three things, and nothing else:

1. **Glyphs.** A cell's `ch` is a CP437 byte. `cp437::to_char` maps it for a
   terminal; a framebuffer host indexes a font atlas with the byte directly.
2. **Colour.** A cell's `attr` is a VGA attribute byte — `fg()` gives 16
   foreground colours, `bg()` gives 8 background colours, `blink()` is bit 7.
3. **Input.** Translate the host's key events into `FormEvent`.

`examples/terminal.rs` is a complete host in roughly 250 lines, most of it
comment. Its `--dump` mode renders one frame to stdout as text with no TTY and
no raw mode, which makes the rendering verifiable in CI rather than only by a
human driving it.

## Decisions taken at extraction

| Decision | Choice | Reasoning |
|---|---|---|
| License | MIT | Simplest permissive licence; GPLv2-compatible. Patent exposure for a cell renderer is nil, so the Apache-2.0 dual form bought nothing here. |
| Visibility | Public | Required for crates.io, docs.rs, and OIDC publishing. |
| Version | 0.1.0 | Fresh crate. Inheriting a version number from an unrelated project would misrepresent its history. |
| Release tooling | release-plz | Conventional commits drive the bump, changelog (via git-cliff), tag, and crates.io publish. Both crates share a `version_group` so they move in lockstep. |
| cargo-dist | Rejected | It ships prebuilt binaries. These are library crates; the demo is `cargo run --example`. Nobody needs a prebuilt copy of it. |
| MSRV | 1.76 | Verified against that toolchain in CI, not merely declared. `core::iter::repeat_n` was replaced with `repeat().take()` rather than let one convenience raise the floor to 1.82. |
| History | Fresh | No history imported, so no lineage from the originating repository survives in commits. |

### CP437 lives in the substrate

The mapping table sits in `neovision-core`, not in a host. Every host needs it,
so putting it in one example would guarantee it was copied into the next. It is
zero-dependency `const` data and costs 512 bytes.

Having it there also removed a real limitation: `CellDraw::write_str` used to
degrade every non-ASCII character to `?` because it could not reach a mapping
table. It now folds through `cp437::from_char`, so a character CP437 can
actually represent — `é`, `½`, the box-drawing glyphs — renders properly, and
only genuinely unrepresentable characters become `?`. One `char` still occupies
exactly one cell, which is what lets callers reason about column alignment.

## Verification

The extraction is mechanically safe because the suite came with it. Every claim
below is checked in CI on each push:

- 146 tests (unit + doctests)
- `cargo clippy --workspace --all-targets -- -D warnings`
- `cargo doc --workspace --no-deps` with `RUSTDOCFLAGS=-D warnings`
- MSRV build against the 1.76 toolchain
- A `thumbv7em-none-eabihf` build, which proves the `no_std` claim by compiling
  for a target that has no operating system at all — a host build never can

## Publishing

Trusted publishing (OIDC), not a long-lived API token. A trusted publisher can
only be registered against a crate that already exists, so the first release is
published manually; afterwards `rust-lang/crates-io-auth-action` exchanges
GitHub's OIDC token for a short-lived crates.io token and revokes it when the
job ends. No publishing secret is ever stored in the repository.

The release job runs in a GitHub `release` environment, and the crates.io
trusted publisher is registered against that environment name. That is a second
claim an attacker must satisfy beyond the pinned workflow filename, and it
restricts deployment to `main` through GitHub rather than through this
workflow's own triggers — so the restriction survives any later change to them.

The environment deliberately carries **no required reviewers**. The release job
runs on every push to `main` and no-ops unless there is something to release, so
an approval rule would prompt on unrelated merges and train whoever holds it to
approve without looking. The genuine human gate is the decision to merge the
release PR; a second gate on the same decision buys nothing. If the repository
later gains collaborators and a real approval step is wanted, the right shape is
to trigger the release job on tag pushes so the prompt fires once per release
rather than once per merge.

## Deliberately out of scope

**Turbo Vision's `TGroup` / `TApplication` layer** — a view tree, a desktop,
stacked and nested modal dialogs, z-ordered windows with focus chains.
neovision models **one modal overlay at a time**.

If that materialises, the growth path is a separate crate (`neovision-app`, the
`TApplication`/desktop analogue) once it carries a view tree and a window stack
rather than a single overlay. At that point a separate crate is justified,
because it is a substantial independently-versioned concern rather than a thin
helper. The widget layer and the substrate should not need to change to support
it: the group/app layer composes the existing `Layer` stack and `FormState`s.

**Trigger condition:** do not build it speculatively. Add it when a real
consumer needs stacked modals or a desktop metaphor. YAGNI until then.

## Resolved

### The framebuffer host, and the font

Built as `examples/framebuffer.rs`. The bundled face lives in
`neovision-core::font` behind a default-off `font` feature: 256 glyphs of 16
rows, one byte per row, most significant bit leftmost — the layout of the VGA
font ROM, so a row byte blits by walking its bits. Feature-gated rather than
unconditional because only pixel hosts need it and 4 KiB is not free on a
microcontroller; in `neovision-core` rather than in the example because every
pixel host needs it, and leaving it in one example guarantees it gets copied
into the next.

The bytes are a dump of an IBM VGA BIOS ROM character generator. Typeface
*designs* are not copyrightable subject matter in the United States
(37 CFR 202.1(e)) and a bitmap face is data rather than a program, which is why
such dumps circulate freely; scalable font *programs* are a different matter and
none are involved. A host wanting its own face declares its own
`[[u8; 16]; 256]` — nothing in the table is privileged.

`--single` renders one frame to a PPM with no display attached, so the
rasterizer is checked in CI rather than only by eye, and `--keys` drives the
form first so states a human would have to tab into — an edit caret, an open
popup — stay reachable headlessly.

### `FormTheme.cursor` — kept, and its default corrected

Writing the pixel host settled it, exactly as intended. The field is real: a
pixel host is the first thing that has to decide what colour a caret is, since
`TextCursor` deliberately carries position and shape but not colour.

Building it also exposed a genuine bug in the default. `FormTheme::DEFAULT`
had `cursor: 0x1F`, bright white — a sensible choice against the blue panel,
and the wrong one, because a caret only ever appears on the **focused row**,
which is painted in `selected` (`0x71`, a light-grey background). White on light
grey is about 1.6:1; the caret was very nearly invisible. The default is now
`0x70`, a black caret, which contrasts against the row it actually lands on.

The field's documentation now states that its foreground nibble is the caret
colour and that it must contrast against `selected` rather than `normal` — the
assumption that caused the bug.

The two shapes resolve colour differently, and both are correct:
`CursorShape::Overtype` is documented as reverse video and so needs nothing
external — swapping the cell's own two colours is the whole of it, matching what
VGA hardware does. `CursorShape::Insert` has no such rule, so it reads the
theme.

## Open items

_None currently tracked._
