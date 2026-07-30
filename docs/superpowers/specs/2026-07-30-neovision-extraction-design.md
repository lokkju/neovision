# neovision — design and extraction record

> **Status:** as-built, and current as of the Turbo Vision parity pass. Records
> what neovision is, how it is structured, and the reasoning behind the
> decisions taken — both at extraction and while closing parity. Items still
> open are marked as such.

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

There are three complete hosts to read. `terminal.rs` hands cells to a
terminal; `framebuffer.rs` rasterizes every pixel itself; `embedded.rs` draws
through `embedded-graphics`' `DrawTarget`, so the same function compiles
unchanged against a real ILI9341 or ST7789 panel.

Every host has a headless mode — `--dump` for the terminal one, `--single` for
the other two — that renders without a display, so what they draw is checked in
CI rather than only by a human driving it. All three also accept `--keys`, which
drives the form before rendering so that states a human would have to tab into
stay reachable.

**Looking at a rendered frame catches what tests do not.** Three separate
rendering defects shipped past green test suites during development and were
each caught in one glance at a dump: a scrollbar drawn over the popup's border,
a scrollbar track drawn in the same glyph as the desktop behind it, and a caret
coloured for a background it never appears on. Render and look before believing
a render test.

## Decisions taken at extraction

| Decision | Choice | Reasoning |
|---|---|---|
| License | MIT | Simplest permissive licence; GPLv2-compatible. Patent exposure for a cell renderer is nil, so the Apache-2.0 dual form bought nothing here. |
| Visibility | Public | Required for crates.io, docs.rs, and OIDC publishing. |
| Version | 0.1.0, then 1.0.0 | Started fresh rather than inheriting a version from an unrelated project. 1.0.0 was cut once the parity pass settled the API; release-plz computes 0.2.0 from a 0.x manifest, since a breaking change bumps the minor below 1.0, so the major was set by hand. |
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

- 226 tests (unit + doctests)
- `cargo clippy --workspace --all-targets -- -D warnings`
- `cargo doc --workspace --no-deps` with `RUSTDOCFLAGS=-D warnings`
- MSRV build against the 1.76 toolchain
- A `thumbv7em-none-eabihf` build, which proves the `no_std` claim by compiling
  for a target that has no operating system at all — a host build never can
- A headless render from each of the three hosts, so the rasterizers are
  covered rather than merely compiled

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

## Turbo Vision parity

The dialog control set was audited against Turbo Vision and closed:

| Turbo Vision | neovision |
|---|---|
| `TInputLine` | `FieldKind::Text`, sharing one edit state machine with `Number` |
| `TButton` | `FieldKind::Button` with a `ButtonRole` and an optional default |
| `TCheckBoxes` | `FieldKind::Cluster` with `ClusterStyle::Check` |
| `TRadioButtons` | `FieldKind::Cluster` with `ClusterStyle::Radio` |
| `TStaticText` | `FieldKind::ReadOnly` |
| `TLabel` + hotkey | `~X~` markers on **button** labels, matched via `FormEvent::Hotkey` |
| `TListBox` + `TScrollBar` | the choice popup, which scrolls and draws a bar |
| — | `FieldKind::Choice`, a dropdown Turbo Vision never had |

Still owed, and deliberately: validators, `TMemo`, `THistory`.

### Keys

> The reasoning behind each of these, with precedents and costs, is recorded in
> [`docs/ux-decisions.md`](../../ux-decisions.md). This section is a summary.

Three traditions disagree about Enter, so a form is told which it belongs to
via [`EnterReach`], defaulting to the conservative `OperateOnly`. Space is
uniform under all of them and always operates the focused control, which is
what keeps any of the three from feeling arbitrary.

Enter opens a dropdown, following BIOS setup rather than Windows — highlight a
setting, press Enter, get its value list. BIOS is also why clusters were not
made the primary picker: BIOS setups have no clusters at all, having solved the
same problem with a popup, which is what `Choice` already is.

Arrows walk straight through a cluster rather than being trapped in it, and Tab
skips the whole thing. Turbo Vision and Windows both trap, but only because an
arrow inside a group *is* their choosing mechanism; arrows only move here, so
the reason does not carry over.

**A radio caret moves without choosing**, which native radio groups do not do.
ARIA's radio pattern, HTML and Windows all move the selection with the caret,
and APG is explicit that this is right when the result is "nearly
instantaneous" — and wrong when choosing "causes a network request" or other
real work, because arrowing through the group then does that work at every
step. For multi-select APG requires Enter or Space outright.

Choosing is never free here: a cluster emits its item's action to the host,
which may reconfigure a display or write to flash, so arrowing past three
options would do it three times. Requiring a keystroke also keeps the two
cluster styles alike under the rule that Space always operates the focused
control, and lets an assistive reader inspect an option without selecting it.

Because caret and choice can then sit on different rows, the renderer marks
them separately: the caret by the selection bar, the choice by the bullet.

### Hotkeys are an inversion

The host detects an accelerator and reports `FormEvent::Hotkey(char)`; the
toolkit matches it, because only the form knows which field claimed the
character and only it knows that a hotkey presses a button but merely focuses
anything else.

A host that cannot deliver hotkeys at all — an embedded keypad, a canvas that
swallows modifiers — simply never emits it, and must also clear `Theme::hotkey`
so that labels stop advertising an affordance nothing will honour. That
requirement is what forced `render_themed` into existence: `Theme` had been
unreachable, with `render_impl` hardcoding `Theme::DEFAULT` despite the type's
own documentation promising that a skin could vary it.

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

### `Theme.cursor` — kept, and its default corrected

Writing the pixel host settled it, exactly as intended. The field is real: a
pixel host is the first thing that has to decide what colour a caret is, since
`TextCursor` deliberately carries position and shape but not colour.

Building it also exposed a genuine bug in the default. `Theme::DEFAULT`
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

- **Validators.** Turbo Vision's `TValidator` family — range, picture, filter,
  string-lookup — applied to an entry field. `Number` clamps to `min`/`max` and
  nothing else validates.
- **`TMemo`.** Multi-line text. The edit state machine is single-line
  throughout, so this is a new field kind rather than a flag on `Text`.
- **`THistory`.** A dropdown of previously entered values on an input line.
