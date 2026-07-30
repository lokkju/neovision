# UX decisions

Why neovision behaves the way it does. Each entry records the decision, the
reasoning, the precedent it follows or departs from, and what it costs — so a
future change is an argument with this document rather than a coin toss.

Several of these depart from a dominant convention. Where they do, the
departure is stated as such and justified, because "we did not know" and "we
decided otherwise" should not look alike a year from now.

## Contents

- [The toolkit describes, the host decides](#the-toolkit-describes-the-host-decides)
- [Keys](#keys)
  - [Space always operates the focused control](#space-always-operates-the-focused-control)
  - [How far Enter reaches is the form's choice](#how-far-enter-reaches-is-the-forms-choice)
  - [Enter opens a dropdown](#enter-opens-a-dropdown)
  - [Only buttons claim accelerators](#only-buttons-claim-accelerators)
  - [Hotkeys are an inversion](#hotkeys-are-an-inversion)
- [Controls](#controls)
  - [The dropdown stays, alongside clusters](#the-dropdown-stays-alongside-clusters)
  - [Arrows walk straight through a cluster](#arrows-walk-straight-through-a-cluster)
  - [A radio caret moves without choosing](#a-radio-caret-moves-without-choosing)
  - [Buttons have roles, not kinds](#buttons-have-roles-not-kinds)
  - [Entry fields select their whole value on focus](#entry-fields-select-their-whole-value-on-focus)
- [Drawing](#drawing)
  - [The caret is reported, never drawn](#the-caret-is-reported-never-drawn)
  - [A caret must contrast against the row it lands on](#a-caret-must-contrast-against-the-row-it-lands-on)
  - [A scrollbar appears only when it means something](#a-scrollbar-appears-only-when-it-means-something)
  - [One char is one cell](#one-char-is-one-cell)
- [Undo](#undo)
  - [Cancel restores only what was touched](#cancel-restores-only-what-was-touched)

---

## The toolkit describes, the host decides

`FormState<A>` carries values of the caller's own action type. It stores them,
hands them back when the user picks something, and never inspects them.

**Why.** A form's job is to say what the user asked for, not to know how to do
it. Keeping `A` opaque is what makes that literal rather than aspirational: the
toolkit *cannot* act on an action, so it cannot accumulate opinions about the
application driving it. It is also the whole of the dependency-purity rule —
there is nothing to depend on.

**Cost.** The host must apply actions itself, and a form cannot react to its own
effects without being rebuilt. `FormState::refresh_fields` exists for that.

---

## Keys

### Space always operates the focused control

Space presses a button, flips a toggle, opens a dropdown, and chooses a cluster
item. The only exception is a text field, where Space has to mean a space.

**Why.** Enter's meaning varies — deliberately, see below — so something must
not vary. Space is the key that always does the obvious thing to whatever is
focused, which is what stops the Enter rules from feeling arbitrary. A user who
learns only Space can drive any form.

**Precedent.** Windows uses Space to activate a focused control, and Turbo
Vision used it for cluster items.

### How far Enter reaches is the form's choice

`EnterReach` has three settings, defaulting to `OperateOnly`:

| | |
|---|---|
| `OperateOnly` | Enter only ever operates the focused control. Accepting means focusing a button. |
| `AcceptWhenIdle` | Enter operates what can be operated; otherwise commits the entry field and presses the default button. |
| `AlwaysAccept` | Enter always presses the default button, except on a focused button. |

**Why three.** Three traditions genuinely disagree, and all three are coherent.
BIOS setup screens use Enter to open a setting and leave with an explicit key.
Turbo Vision and Windows dialogs give Enter to the default button. Picking one
would be picking which kind of program neovision is for, which is not the
toolkit's call.

**Why `OperateOnly` by default.** It is the conservative reading: a form that
does not expect Enter to close it never will, whatever buttons it happens to
carry. Opting into dialog behaviour is one call; discovering that Enter
silently dismissed your form is not recoverable.

**Invariant across all three.** A focused button is pressed. Tabbing to Cancel
and pressing Enter must press Cancel, never reach past it to the default.

### Enter opens a dropdown

Highlight a `Choice`, press Enter, get its value list.

**Why.** This is the BIOS-setup habit — AMI, Award and Phoenix all work this
way — and it is the tradition a text-mode toolkit most resembles. Windows
dialogs instead reserve Enter for the default button and open combo boxes with
Alt+Down.

**Departure.** From Windows/CUA, knowingly. The cost would normally be an
inconsistency — Enter meaning "open" here and "accept" there — but Space
operating everything means no one has to remember which. That is what makes the
departure affordable.

### Only buttons claim accelerators

A `~X~` marker is live in a button's label. In a field label or a cluster item
it is stripped and ignored — the text renders correctly and promises nothing.

**Why not everywhere.** Turbo Vision put accelerators on labels bound to
controls, and it is tempting: jumping to `Frame delay` beats seven tabs. But
labels are domain words, not letters chosen to be distinct, so a form of any
size collides. A `Sound` field and an `OK` button both claim `o`; first match
wins; **the button becomes unreachable** — losing the one accelerator that does
something Tab cannot. That is not a hypothetical, it is what the demo did.

The rule that falls out: **an accelerator invokes a command; navigation is
arrows and Tab.** Navigation needs no namespace and cannot collide.

**Why now rather than later.** Widening this — to menu items, once there are
menus — is additive and breaks nothing. Narrowing it would break every form
relying on it. A scope decision that is cheap in one direction and expensive in
the other should start at the cheap end.

**Consequence.** Nothing advertises what it cannot do: a field label never
draws a marked letter, because it would be promising an accelerator that is not
there. `FormState::hotkey_conflicts` reports duplicate claims so a form's own
tests can catch them, since only the form's author can resolve one.

### Hotkeys are an inversion

The host detects an accelerator and reports `FormEvent::Hotkey(char)`. The
toolkit decides what it means.

**Why split it there.** Only the form knows which field claimed the character,
and only the form knows the CUA rule that an accelerator presses a button but
merely focuses anything else. Pushing that into hosts means every host
reimplements it, and they drift.

**Why the host must opt in.** Plenty of hosts have no Alt at all — an embedded
keypad, a game reading scancodes, a canvas that captures modifiers. A host that
never emits `Hotkey` simply has no accelerators and everything else still
works.

**The consequence that is easy to miss.** A host that cannot deliver hotkeys
must also clear `Theme::hotkey`, or labels advertise an affordance nothing will
honour. Making that possible is what forced `render_themed` into existence:
`Theme` had been unreachable, with the renderer hardcoding `Theme::DEFAULT`
despite the type's own documentation promising a skin could vary it.

---

## Controls

### The dropdown stays, alongside clusters

`Choice` is a dropdown. Turbo Vision had no such control.

**Why.** Parity is not a reason to regress. A dropdown is better than a radio
cluster when there are many options or vertical space is short; a cluster is
better for two to four options that should all be visible. Both belong, and
BIOS setups are the proof that a popup value list is a perfectly good primary
picker — they use nothing else.

### Arrows walk straight through a cluster

Up and Down move through a cluster's items and carry on to the next field. Tab
skips the whole cluster.

**Departure.** Turbo Vision and Windows both trap arrows inside a group.

**Why depart.** They trap for a reason that does not apply here. In both, an
arrow key inside a group *is* the choosing mechanism — which is exactly why the
group has to own the key, and why a radio group is one tab stop. Arrows only
move here (see [above](#a-radio-caret-moves-without-choosing)), and a key that
only moves has no business stopping at a boundary the user cannot see a reason
for. Turbo Vision's trapping was additionally an artifact of a cluster being one
view in a `TGroup` tree, machinery this has no equivalent of.

What is left is a form that reads as one vertical list of rows and behaves like
one, which is also what BIOS setup screens do — arrows traverse continuously and
nothing is ever trapped.

**Consequence.** Arrowing *into* a cluster puts the caret at the end it arrived
from: down-arrow lands on the first item, up-arrow on the last. Otherwise
continuing in one direction would jump to wherever the caret happened to sit.

**Note.** This decision and the one above are load-bearing on each other. If a
radio caret is ever made to choose as it moves again, trapping has to come
back with it.

### A radio caret moves without choosing

Moving the caret through a radio cluster changes nothing. Space or Enter
chooses what the caret is on.

**Departure, and a large one.** ARIA's radio pattern, native HTML radios and
Windows all move the *selection* with the caret.

**Why depart.** [APG's own guidance][apg-focus] qualifies that rule: selection
should follow focus when the result is "nearly instantaneous", and should not
when it "causes a network request" or comparable work, because arrowing through
the group then does that work at every step. For multi-select it requires Enter
or Space outright.

Choosing is never free here. A cluster emits its item's action to the host,
which may reconfigure a display or write to flash — on the embedded target,
literally. Arrowing past three options would do that three times, and `Escape`
is not an undo for work already performed.

**What it also buys.** The two cluster styles behave identically, so "arrows
move, Space operates" holds everywhere. An assistive reader can inspect an
option without selecting it, which the [Primer team names][primer] as the cost
of selection-follows-focus: "screen readers can only read an option by
selecting it."

**Consequence.** Caret and choice can sit on different rows, so the renderer
marks them separately — the caret by the selection bar, the choice by the
bullet.

[apg-focus]: https://www.w3.org/WAI/ARIA/apg/practices/keyboard-interface/
[primer]: https://primer.style/product/components/radio-group/accessibility/

### Buttons have roles, not kinds

A button carries a label, a `ButtonRole` (`Accept` / `Reject` / `Stay`), an
optional action, and whether it is the default.

**Why.** The old `ButtonKind::{Ok, Cancel}` encoded two labels, but what those
two names actually meant was their *effect on the form* — keep changes and
close, restore and close. Naming the effect lets a form have Save, Discard,
Apply and Help without pretending two of them are OK and Cancel.

**Drawing.** The default button is drawn with CP437 guillemets rather than
square brackets — `«  OK  »` — which is the same width, so nothing shifts.

### Entry fields select their whole value on focus

Focus lands on a `Text` or `Number` field with its value selected; the first
character typed replaces it.

**Why.** Turbo Vision's "selected on entry". The common case for an entry field
holding a default is replacing it, and this makes that one keystroke while
still allowing in-place editing — any navigation key drops the selection and
leaves the caret where you put it.

---

## Drawing

### The caret is reported, never drawn

`render_with_cursor` returns a `TextCursor` describing position and shape. It is
never written into the cell buffer.

**Why.** A caret drawn into the buffer overwrites the glyph beneath it — the
character you are editing. Reporting it lets the host draw it as scanlines, as
a terminal cursor, or by inverting the cell, none of which destroy anything.

**Consequence.** `TextCursor` deliberately carries no colour, since position and
shape are facts about the form while colour is a fact about the skin.

### A caret must contrast against the row it lands on

`Theme::cursor` is documented as needing to contrast against `selected`, not
`normal`.

**Why it is worth stating.** The default was originally bright white, chosen
against the blue panel — sensible, and wrong, because a caret only ever appears
on the focused row, which is painted in the selection attribute. White on light
grey is about 1.6:1 and the caret was effectively invisible. The same mistake
was then made twice more, on the hotkey attribute and the scrollbar track.

The general form: **an attribute has to contrast against the background it
actually appears on, which is often not the one it was picked beside.**

### A scrollbar appears only when it means something

The choice popup draws a bar only when the list does not fit.

**Why.** A bar that is always full says only that there is nothing to scroll,
which the absence of a bar says more quietly. Turbo Vision always drew one; this
does not.

**Two things it must not do**, both learned by looking at a rendered frame after
the tests were green: it must not be drawn on the frame's border column, which
leaves the popup looking like a box with a hole in it; and its track must not
use the same glyph as the desktop fill, or the track vanishes wherever the popup
overhangs. The bar gets its own column, and the track is medium shade against
the desktop's light.

### One char is one cell

`CellDraw::write_str` folds text through the CP437 table and writes exactly one
cell per `char`. Anything CP437 cannot represent becomes `?`.

**Why.** Column alignment is the whole basis of a cell grid. A renderer that
sometimes consumed two cells for one character would make every width
calculation conditional. Folding rather than rejecting means `é` and `½` and the
box-drawing glyphs all render properly instead of being degraded.

**Related.** A text field's caret counts *characters*, not bytes, for the same
reason: one char is one column. `Number` got away with byte indices because
digits are ASCII; `Text` cannot.

---

## Undo

### Cancel restores only what was touched

Each field carries the actions that restore its value as it was when the form
opened. Cancel replays those for the fields the user actually changed, and only
those.

**Why not replay everything.** Each restore action looks like a no-op for an
untouched field, but an action can carry effects beyond its own field. Replaying
one the user never asked for is a change, not a restoration.

**Why a `Vec` rather than one action.** Restoring a value is not always the same
operation as choosing it. An action that selects a mode may also clobber a
related setting, so undoing it takes two actions — one for the mode, one for
what it clobbered.
