//! A modal form: a list of fields, one focused, optionally with an open
//! choice popup.
//!
//! Generic over the caller's action type `A`. The model stores and returns
//! `A` values but never inspects them, so it carries no knowledge of the
//! application it drives. It also never mutates anything — [`FormState::handle`]
//! returns the actions it wants applied and the caller applies them, which
//! keeps a single mutation path in the host.

use alloc::string::String;
use alloc::vec::Vec;

/// How far Enter reaches.
///
/// Three traditions disagree about this, and all three are coherent, so the
/// form is told which one it belongs to rather than the toolkit picking.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum EnterReach {
    /// Enter only ever operates the focused control. Accepting the form means
    /// focusing a button and pressing it.
    ///
    /// The BIOS-setup habit: Enter opens a setting's value list, and you leave
    /// the screen with an explicit key. The default, because it is the
    /// conservative reading — a form that does not expect Enter to close it
    /// never will.
    #[default]
    OperateOnly,
    /// Enter operates a control that has something to operate, and otherwise
    /// commits the entry field and presses the default button.
    ///
    /// The middle reading: typing a value and pressing Enter finishes the
    /// dialog, but Enter on a dropdown still opens it.
    AcceptWhenIdle,
    /// Enter always presses the default button, except on a focused button,
    /// which it presses instead. Nothing else consumes it.
    ///
    /// Turbo Vision's own rule, and Windows'. A dropdown then needs Space.
    AlwaysAccept,
}

/// What pressing a button does to the form.
///
/// The distinction Ok and Cancel actually encoded was never their labels — it
/// was this. Naming it lets a form have "Save", "Discard" and "Help" without
/// pretending two of them are OK and Cancel.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ButtonRole {
    /// Close, keeping what the user changed. What OK does.
    Accept,
    /// Close, replaying the restore actions of every field the user touched.
    /// What Cancel does, and what Escape does.
    Reject,
    /// Emit the action and leave the form open — Apply, Help, Reset.
    Stay,
}

/// One selectable value of a [`FieldKind::Choice`].
#[derive(Debug, Clone)]
pub struct ChoiceOption<A> {
    pub label: String,
    pub action: A,
}

/// Split a label on Turbo Vision's `~X~` mnemonic markers.
///
/// Returns the text as it should be drawn (tildes removed) together with the
/// mnemonic character and its position in that text. `~O~pen` yields
/// `("Open", Some(('O', 0)))`; a label with no markers yields no mnemonic.
///
/// Only the first marked character counts — a label claiming two accelerators
/// is a mistake, and taking the first is more predictable than taking the last.
pub fn parse_mnemonic(label: &str) -> (String, Option<(char, usize)>) {
    let mut text = String::with_capacity(label.len());
    let mut found: Option<(char, usize)> = None;
    let mut marked = false;
    for ch in label.chars() {
        if ch == '~' {
            marked = !marked;
            continue;
        }
        if marked && found.is_none() {
            found = Some((ch, text.chars().count()));
        }
        text.push(ch);
    }
    (text, found)
}

/// The character a label claims as its accelerator, if any.
fn mnemonic_of(label: &str) -> Option<char> {
    parse_mnemonic(label).1.map(|(c, _)| c)
}

/// A borrowed view of whatever entry field currently has focus.
///
/// Both [`FieldKind::Text`] and [`FieldKind::Number`] are entry fields: they
/// hold a buffer, a caret, a whole-value selection and an insert/overtype
/// mode, and they respond to typing identically apart from what they accept
/// and how long they may get. Turbo Vision drew the same line — numeric entry
/// was a validated `TInputLine`, not a separate control.
///
/// Borrowing them into one shape means the editing state machine is written
/// and tested once instead of once per field kind.
struct EditRef<'a> {
    buffer: &'a mut String,
    /// Caret position in **characters**, not bytes. One char is one cell, so
    /// this is also the caret's column offset within the value.
    cursor: &'a mut usize,
    selected: &'a mut bool,
    overtype: &'a mut bool,
    /// Longest the buffer may become, in characters.
    max_len: usize,
    /// Whether the field takes only ASCII digits.
    digits_only: bool,
}

impl EditRef<'_> {
    /// Character count, which is also the caret's maximum position.
    fn len(&self) -> usize {
        self.buffer.chars().count()
    }

    /// Byte offset of a character index, for the places `String` needs one.
    fn byte_of(&self, char_idx: usize) -> usize {
        self.buffer
            .char_indices()
            .nth(char_idx)
            .map(|(b, _)| b)
            .unwrap_or(self.buffer.len())
    }

    fn type_char(&mut self, c: char) {
        if self.digits_only && !c.is_ascii_digit() {
            return;
        }
        if *self.selected {
            self.buffer.clear();
            self.buffer.push(c);
            *self.selected = false;
            *self.cursor = 1;
            return;
        }
        let len = self.len();
        if *self.overtype && *self.cursor < len {
            let start = self.byte_of(*self.cursor);
            let end = self.byte_of(*self.cursor + 1);
            let mut tmp = [0u8; 4];
            self.buffer
                .replace_range(start..end, c.encode_utf8(&mut tmp));
            *self.cursor += 1;
        } else if len < self.max_len {
            let at = self.byte_of(*self.cursor);
            self.buffer.insert(at, c);
            *self.cursor += 1;
        }
    }

    fn backspace(&mut self) {
        if *self.selected {
            self.buffer.clear();
            *self.selected = false;
            *self.cursor = 0;
        } else if *self.cursor > 0 {
            let at = self.byte_of(*self.cursor - 1);
            self.buffer.remove(at);
            *self.cursor -= 1;
        }
    }

    fn delete(&mut self) {
        if *self.selected {
            self.buffer.clear();
            *self.selected = false;
            *self.cursor = 0;
        } else if *self.cursor < self.len() {
            let at = self.byte_of(*self.cursor);
            self.buffer.remove(at);
        }
    }

    fn home(&mut self) {
        *self.selected = false;
        *self.cursor = 0;
    }

    fn end(&mut self) {
        *self.selected = false;
        *self.cursor = self.len();
    }

    fn left(&mut self) {
        if *self.selected {
            *self.selected = false;
            *self.cursor = 0;
        } else {
            *self.cursor = self.cursor.saturating_sub(1);
        }
    }

    fn right(&mut self) {
        // Right when selected moves the caret to the end (CUA spec), rather
        // than relying on the unstated invariant that a selected field's
        // caret already sits there.
        if *self.selected {
            *self.selected = false;
            *self.cursor = self.len();
        } else {
            *self.cursor = (*self.cursor + 1).min(self.len());
        }
    }

    fn toggle_overtype(&mut self) {
        *self.overtype = !*self.overtype;
        *self.selected = false;
    }

    fn select_all(&mut self) {
        *self.selected = true;
        *self.cursor = self.len();
    }
}

/// What a field is and how it behaves.
///
/// Marked non-exhaustive: new field kinds are expected, and a consumer's
/// `match` should not break every time one arrives.
#[derive(Debug, Clone)]
#[non_exhaustive]
pub enum FieldKind<A> {
    /// One-of-N. Enter opens a popup listing every option.
    Choice {
        options: Vec<ChoiceOption<A>>,
        /// `None` means nothing is selected — the value this field reflects is
        /// genuinely unset, not merely absent from `options`. A field whose
        /// option list is a curated subset of a larger domain must instead
        /// ensure the current value appears among `options` before the field
        /// is constructed, so that "unset" keeps its meaning.
        selected: Option<usize>,
    },
    /// Bounded integer entry. Digits buffer; Enter parses, clamps and commits.
    ///
    /// `commit` is a plain fn pointer rather than a pre-built action because
    /// the action depends on the value typed, which is unknown at
    /// construction. A fn pointer keeps this `Clone` and avoids boxing in
    /// `no_std`.
    Number {
        value: u32,
        /// The live digit string: seeded from `value` and edited in place.
        buffer: String,
        /// Insertion point within `buffer`, in `0..=buffer.len()`.
        cursor: usize,
        /// Whole-value selection (Turbo Vision "selected on entry"). While set,
        /// the first digit replaces the buffer; any navigation/edit key clears
        /// it and drops into in-place editing.
        selected: bool,
        /// `false` = insert (default), `true` = overtype.
        overtype: bool,
        min: u32,
        max: u32,
        unit: &'static str,
        commit: fn(u32) -> A,
    },
    /// Free text entry — the analogue of Turbo Vision's `TInputLine`.
    ///
    /// Typing buffers; Enter commits the buffer through `commit`. Editing
    /// behaves exactly as [`FieldKind::Number`] does, because both borrow the
    /// same state machine: whole-value selection on entry, insert/overtype
    /// toggled by Insert, Home/End/Delete/Backspace as CUA specifies.
    Text {
        /// The live text, edited in place.
        buffer: String,
        /// Caret position in **characters**, in `0..=buffer.chars().count()`.
        /// One char renders as one cell, so this is also its column offset.
        cursor: usize,
        /// Whole-value selection (Turbo Vision "selected on entry"). While
        /// set, the first character typed replaces the whole buffer.
        selected: bool,
        /// `false` = insert (default), `true` = overtype.
        overtype: bool,
        /// Longest the buffer may become, in characters.
        max_len: usize,
        /// Called with the committed text to build the caller's action.
        ///
        /// A plain fn pointer rather than a closure, for the same reason
        /// [`FieldKind::Number`] uses one: it keeps the field `Clone` and
        /// avoids boxing under `no_std`.
        commit: fn(&str) -> A,
    },
    /// Two-state. Enter flips it and emits the matching action.
    Toggle {
        on: bool,
        on_action: A,
        off_action: A,
    },
    /// A derived value, shown but not editable. Skipped by focus traversal.
    ReadOnly(String),
    /// A push button.
    ///
    /// Its `label` may mark an accelerator with `~X~`, exactly as a field
    /// label does — there is no separate mechanism for buttons.
    Button {
        label: &'static str,
        role: ButtonRole,
        /// Emitted when pressed, before the form closes if it is going to.
        action: Option<A>,
        /// Whether Enter presses this button from anywhere in the form.
        ///
        /// Turbo Vision's rule, and everyone's expectation of a dialog: fill
        /// the fields in, press Enter, the dialog accepts. Only the first
        /// button claiming it counts.
        default: bool,
    },
}

/// A labelled field.
#[derive(Debug, Clone)]
pub struct Field<A> {
    pub label: &'static str,
    pub kind: FieldKind<A>,
    /// The actions that restore this field's value as it was when the form
    /// opened, applied in order. An empty vector means the field contributes
    /// nothing to Cancel.
    ///
    /// Deliberately separate from the field's own selection action: restoring
    /// a value is not always the same operation as choosing it. A toggle whose
    /// "on" action has side effects beyond its own field cannot be undone by
    /// its "off" action, so the caller supplies the action(s) that genuinely
    /// put the underlying state back. A single `A` cannot always express that
    /// restoration: an action that selects a mode may also clobber a related
    /// setting as a side effect, so undoing it takes two actions — one for the
    /// mode, one for what it clobbered. Hence `Vec` rather than `Option`.
    pub restore: Vec<A>,
}

impl<A> Field<A> {
    /// The standard accepting button: closes the form, keeping changes.
    ///
    /// Its label marks `O` as the accelerator, as CUA dialogs always have.
    pub fn ok() -> Self {
        Self {
            label: "",
            kind: FieldKind::Button {
                label: "~O~K",
                role: ButtonRole::Accept,
                action: None,
                default: true,
            },
            restore: Vec::new(),
        }
    }

    /// The standard rejecting button: closes the form, restoring every field
    /// the user changed — the same thing Escape does.
    pub fn cancel() -> Self {
        Self {
            label: "",
            kind: FieldKind::Button {
                label: "~C~ancel",
                role: ButtonRole::Reject,
                action: None,
                default: false,
            },
            restore: Vec::new(),
        }
    }

    /// A button with the caller's own label, role and action.
    ///
    /// Mark its accelerator in `label` with `~X~`, exactly as for a field
    /// label — buttons have no separate mechanism.
    pub fn button(label: &'static str, role: ButtonRole, action: Option<A>) -> Self {
        Self {
            label: "",
            kind: FieldKind::Button {
                label,
                role,
                action,
                default: false,
            },
            restore: Vec::new(),
        }
    }

    /// Make this button the one Enter presses from anywhere in the form.
    ///
    /// Panics if the field is not a button, since there is nothing sensible
    /// to do with the request otherwise.
    pub fn as_default(mut self) -> Self {
        match &mut self.kind {
            FieldKind::Button { default, .. } => *default = true,
            _ => panic!("as_default() is only meaningful on a button"),
        }
        self
    }

    /// False for [`FieldKind::ReadOnly`], which focus traversal skips.
    pub fn focusable(&self) -> bool {
        !matches!(self.kind, FieldKind::ReadOnly(_))
    }
}

/// An open choice list.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Popup {
    /// Index of the field this popup belongs to.
    pub field: usize,
    /// Index of the highlighted option.
    pub highlight: usize,
}

/// A key event, already translated out of the host's input representation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FormEvent {
    /// Move focus up one *row*. Fields are a vertical stack except a trailing
    /// run of buttons (the OK/Cancel bar), which share one row — so Up/Down
    /// step between rows and never between two side-by-side buttons, matching
    /// CUA.
    Up,
    /// Move focus down one row. See [`FormEvent::Up`].
    Down,
    /// Move focus left *within the current row*. Meaningful only where a row
    /// holds more than one field — the button bar — where it moves between the
    /// buttons. A no-op on single-field rows.
    Left,
    /// Move focus right within the current row. See [`FormEvent::Left`].
    Right,
    /// Move focus to the next field in order, across rows (Tab). The primary
    /// control-to-control key, as in Turbo Vision.
    Tab,
    /// Move focus to the previous field in order (Shift+Tab).
    BackTab,
    Enter,
    Escape,
    Char(char),
    Backspace,
    Home,
    End,
    Delete,
    Insert,
    /// The user invoked the accelerator for this character.
    ///
    /// Deliberately named for intent rather than for a key: a desktop host
    /// will map Alt+letter onto it, but an embedded keypad or a game reading
    /// scancodes may have no Alt at all, and is free never to emit this. A
    /// form whose host stays silent simply has no hotkeys, and everything
    /// else still works.
    ///
    /// Matching is the toolkit's job, not the host's — only the form knows
    /// which field claimed the character, and only it knows the CUA rule that
    /// a hotkey on a button presses it while one on any other field focuses
    /// it. Hosts that cannot offer hotkeys should also clear
    /// [`Theme::hotkey`](crate::Theme::hotkey), so labels stop
    /// advertising them.
    Hotkey(char),
}

/// What the host should do after an event.
#[derive(Debug, Clone)]
pub struct FormOutcome<A> {
    /// Actions to apply, in order. Usually empty or one; Cancel emits the
    /// restore action of every field the user actually changed.
    pub actions: Vec<A>,
    /// Whether the form should close.
    pub close: bool,
}

impl<A> FormOutcome<A> {
    fn nothing() -> Self {
        Self {
            actions: Vec::new(),
            close: false,
        }
    }

    fn action(a: A) -> Self {
        Self {
            actions: alloc::vec![a],
            close: false,
        }
    }

    fn closing(actions: Vec<A>) -> Self {
        Self {
            actions,
            close: true,
        }
    }
}

/// A form and its current interaction state.
#[derive(Debug, Clone)]
pub struct FormState<A> {
    pub title: &'static str,
    fields: Vec<Field<A>>,
    focus: usize,
    popup: Option<Popup>,
    /// Each field's `restore` actions as captured when the form opened.
    /// Parallel to `fields`.
    originals: Vec<Vec<A>>,
    /// How far Enter reaches. See [`EnterReach`].
    enter_reach: EnterReach,
    /// Whether the user has changed each field since the form opened.
    /// Parallel to `fields`.
    ///
    /// Cancel restores only dirty fields. Replaying every field's value
    /// instead would be wrong even though each action looks like a no-op:
    /// actions the user never asked for can carry side effects beyond their
    /// own field, so an untouched field must stay untouched.
    dirty: Vec<bool>,
}

impl<A> FormState<A> {
    pub fn focus(&self) -> usize {
        self.focus
    }

    pub fn set_focus(&mut self, index: usize) {
        if index < self.fields.len() && self.is_focusable(index) {
            self.focus = index;
        }
    }

    pub fn fields(&self) -> &[Field<A>] {
        &self.fields
    }

    pub fn popup(&self) -> Option<&Popup> {
        self.popup.as_ref()
    }

    fn is_focusable(&self, index: usize) -> bool {
        self.fields
            .get(index)
            .map(|f| f.focusable())
            .unwrap_or(false)
    }

    /// Move focus by `delta`, wrapping and skipping non-focusable fields. This
    /// is Tab / Shift+Tab: linear order across the whole form, buttons included.
    fn step_focus(&mut self, delta: isize) {
        let n = self.fields.len();
        if n == 0 {
            return;
        }
        let mut i = self.focus;
        // At most `n` steps: if nothing is focusable we stop where we started.
        for _ in 0..n {
            i = ((i as isize + delta).rem_euclid(n as isize)) as usize;
            if self.is_focusable(i) {
                self.focus = i;
                return;
            }
        }
    }

    /// The focusable fields grouped into rows for arrow navigation.
    ///
    /// Every focusable field is its own row, except a trailing run of
    /// consecutive [`FieldKind::Button`] fields, which share one horizontal row
    /// — the same grouping the renderer uses to lay OK/Cancel side by side. So
    /// Up/Down move between rows and Left/Right move within a row.
    fn focus_rows(&self) -> Vec<Vec<usize>> {
        let n = self.fields.len();
        let mut btn_start = n;
        while btn_start > 0 && matches!(self.fields[btn_start - 1].kind, FieldKind::Button { .. }) {
            btn_start -= 1;
        }
        let mut rows = Vec::new();
        for i in 0..btn_start {
            if self.fields[i].focusable() {
                rows.push(alloc::vec![i]);
            }
        }
        if btn_start < n {
            let bar: Vec<usize> = (btn_start..n).filter(|&i| self.is_focusable(i)).collect();
            if !bar.is_empty() {
                rows.push(bar);
            }
        }
        rows
    }

    /// `(row, column)` of the current focus within [`Self::focus_rows`].
    fn focus_pos(&self, rows: &[Vec<usize>]) -> Option<(usize, usize)> {
        rows.iter()
            .enumerate()
            .find_map(|(r, row)| row.iter().position(|&i| i == self.focus).map(|c| (r, c)))
    }

    /// Move focus one row (Up/Down), preserving the column where the target row
    /// is narrower by clamping to its last field. Wraps top-to-bottom.
    fn step_row(&mut self, delta: isize) {
        let rows = self.focus_rows();
        if rows.is_empty() {
            return;
        }
        let (r, c) = match self.focus_pos(&rows) {
            Some(p) => p,
            None => {
                self.focus = rows[0][0];
                return;
            }
        };
        let tr = ((r as isize + delta).rem_euclid(rows.len() as isize)) as usize;
        let row = &rows[tr];
        self.focus = row[c.min(row.len() - 1)];
    }

    /// Move focus within the current row (Left/Right), wrapping. A no-op on a
    /// single-field row.
    fn step_col(&mut self, delta: isize) {
        let rows = self.focus_rows();
        let (r, c) = match self.focus_pos(&rows) {
            Some(p) => p,
            None => return,
        };
        let row = &rows[r];
        let tc = ((c as isize + delta).rem_euclid(row.len() as isize)) as usize;
        self.focus = row[tc];
    }
}

// `A: Clone` is only needed where a stored action is actually cloned out
// (the Cancel snapshot, and handing an action back to the caller on
// activation). The read-only accessors above stay unconstrained so a pure
// consumer — the renderer in `render.rs` — never needs to require `Clone`
// on its own action type just to read a label, the focus index, or the
// open popup.
impl<A: Clone> FormState<A> {
    /// Build a form, capturing each field's `restore` action as its opening
    /// value.
    pub fn new(title: &'static str, fields: Vec<Field<A>>) -> Self {
        let originals: Vec<Vec<A>> = fields.iter().map(|f| f.restore.clone()).collect();
        let dirty = alloc::vec![false; fields.len()];
        // Start on the first focusable field. A form with no focusable field
        // at all is degenerate — focus stays at 0 and every activation is a
        // no-op, which is what the ReadOnly and empty-field arms already do.
        let focus = fields.iter().position(|f| f.focusable()).unwrap_or(0);
        Self {
            title,
            fields,
            focus,
            popup: None,
            originals,
            dirty,
            enter_reach: EnterReach::default(),
        }
    }

    /// Choose how far Enter reaches. See [`EnterReach`].
    pub fn with_enter_reach(mut self, reach: EnterReach) -> Self {
        self.enter_reach = reach;
        self
    }

    /// How far Enter currently reaches.
    pub fn enter_reach(&self) -> EnterReach {
        self.enter_reach
    }

    /// Swap in a freshly built field list without disturbing the interaction.
    ///
    /// The host rebuilds the fields from live state after every applied action,
    /// because one field's action can change another field's value (applying a
    /// preset rewrites colours, density and speed at once). Only the displayed
    /// values are replaced: `focus`, `popup`, `originals` and `dirty` all
    /// survive. `originals` in particular must NOT be recaptured — they are
    /// the values the form opened with, which is exactly what Cancel restores.
    pub fn refresh_fields(&mut self, fields: Vec<Field<A>>) {
        self.fields = fields;
    }

    /// The restore actions of every field the user changed, in field order,
    /// preserving each field's own internal restore order.
    fn cancel_actions(&self) -> Vec<A> {
        let mut out = Vec::new();
        for (i, dirty) in self.dirty.iter().enumerate() {
            if !*dirty {
                continue;
            }
            if let Some(actions) = self.originals.get(i) {
                out.extend(actions.iter().cloned());
            }
        }
        out
    }

    /// Record that field `i` was changed by the user. Out-of-range indices are
    /// ignored rather than panicking: `refresh_fields` may be handed a list of
    /// a different length, and a mismatched index is not worth a crash.
    fn mark_dirty(&mut self, i: usize) {
        if let Some(d) = self.dirty.get_mut(i) {
            *d = true;
        }
    }

    /// Feed one event. Returns the actions to apply and whether to close.
    pub fn handle(&mut self, ev: FormEvent) -> FormOutcome<A> {
        if self.popup.is_some() {
            return self.handle_popup(ev);
        }
        match ev {
            FormEvent::Up => {
                self.step_row(-1);
                self.reselect_focused();
                FormOutcome::nothing()
            }
            FormEvent::Down => {
                self.step_row(1);
                self.reselect_focused();
                FormOutcome::nothing()
            }
            FormEvent::Left => {
                if self.focused_is_entry() {
                    self.on_left();
                } else {
                    self.step_col(-1);
                }
                FormOutcome::nothing()
            }
            FormEvent::Right => {
                if self.focused_is_entry() {
                    self.on_right();
                } else {
                    self.step_col(1);
                }
                FormOutcome::nothing()
            }
            FormEvent::Tab => {
                self.step_focus(1);
                self.reselect_focused();
                FormOutcome::nothing()
            }
            FormEvent::BackTab => {
                self.step_focus(-1);
                self.reselect_focused();
                FormOutcome::nothing()
            }
            FormEvent::Hotkey(c) => self.on_hotkey(c),
            FormEvent::Escape => FormOutcome::closing(self.cancel_actions()),
            FormEvent::Enter => self.activate(),
            FormEvent::Char(' ') => self.on_space(),
            FormEvent::Char(c) => {
                self.type_char(c);
                FormOutcome::nothing()
            }
            FormEvent::Backspace => {
                self.on_backspace();
                FormOutcome::nothing()
            }
            FormEvent::Home => {
                self.on_home();
                FormOutcome::nothing()
            }
            FormEvent::End => {
                self.on_end();
                FormOutcome::nothing()
            }
            FormEvent::Delete => {
                self.on_delete();
                FormOutcome::nothing()
            }
            FormEvent::Insert => {
                self.on_insert();
                FormOutcome::nothing()
            }
        }
    }

    fn focused_kind_mut(&mut self) -> Option<&mut FieldKind<A>> {
        self.fields.get_mut(self.focus).map(|f| &mut f.kind)
    }

    /// The field claiming `c` as its accelerator, if any.
    ///
    /// Case-insensitive, because a host reporting Alt+O has no idea whether
    /// the label spelled it `O` or `o`. Non-focusable fields are skipped: a
    /// read-only row cannot take focus, so letting it claim a character would
    /// silently swallow the accelerator.
    fn hotkey_target(&self, c: char) -> Option<usize> {
        let wanted = c.to_ascii_lowercase();
        self.fields.iter().position(|f| {
            if !f.focusable() {
                return false;
            }
            // A button carries its accelerator in its own label; every other
            // field carries it in the label beside it.
            let claimed = match &f.kind {
                FieldKind::Button { label, .. } => mnemonic_of(label),
                _ => mnemonic_of(f.label),
            };
            claimed.map(|m| m.to_ascii_lowercase()) == Some(wanted)
        })
    }

    /// Focus the field claiming `c` — and, if it is a button, press it.
    ///
    /// That split is the CUA rule: an accelerator on a button activates it,
    /// while one on any other control only moves focus there.
    fn on_hotkey(&mut self, c: char) -> FormOutcome<A> {
        let Some(target) = self.hotkey_target(c) else {
            return FormOutcome::nothing();
        };
        self.focus = target;
        self.reselect_focused();
        if matches!(self.fields[target].kind, FieldKind::Button { .. }) {
            self.activate()
        } else {
            FormOutcome::nothing()
        }
    }

    /// Whether focus is on an entry field, where Left/Right move the caret
    /// rather than moving between fields on a shared row.
    fn focused_is_entry(&self) -> bool {
        matches!(
            self.fields.get(self.focus).map(|f| &f.kind),
            Some(FieldKind::Number { .. }) | Some(FieldKind::Text { .. })
        )
    }

    /// Borrow whichever entry field has focus, if any.
    fn focused_edit(&mut self) -> Option<EditRef<'_>> {
        match self.focused_kind_mut()? {
            FieldKind::Number {
                buffer,
                cursor,
                selected,
                overtype,
                ..
            } => Some(EditRef {
                buffer,
                cursor,
                selected,
                overtype,
                // The digit slot the renderer reserves is four wide.
                max_len: 4,
                digits_only: true,
            }),
            FieldKind::Text {
                buffer,
                cursor,
                selected,
                overtype,
                max_len,
                ..
            } => {
                let max_len = *max_len;
                Some(EditRef {
                    buffer,
                    cursor,
                    selected,
                    overtype,
                    max_len,
                    digits_only: false,
                })
            }
            _ => None,
        }
    }

    /// Re-select an entry field's whole value when focus lands on it, which is
    /// what Turbo Vision did so that typing replaces rather than appends.
    fn reselect_focused(&mut self) {
        if let Some(mut edit) = self.focused_edit() {
            edit.select_all();
        }
    }

    fn type_char(&mut self, c: char) {
        if let Some(mut edit) = self.focused_edit() {
            edit.type_char(c);
        }
    }

    fn on_left(&mut self) {
        if let Some(mut edit) = self.focused_edit() {
            edit.left();
        }
    }

    fn on_right(&mut self) {
        if let Some(mut edit) = self.focused_edit() {
            edit.right();
        }
    }

    fn on_home(&mut self) {
        if let Some(mut edit) = self.focused_edit() {
            edit.home();
        }
    }

    fn on_end(&mut self) {
        if let Some(mut edit) = self.focused_edit() {
            edit.end();
        }
    }

    fn on_backspace(&mut self) {
        if let Some(mut edit) = self.focused_edit() {
            edit.backspace();
        }
    }

    fn on_delete(&mut self) {
        if let Some(mut edit) = self.focused_edit() {
            edit.delete();
        }
    }

    fn on_insert(&mut self) {
        if let Some(mut edit) = self.focused_edit() {
            edit.toggle_overtype();
        }
    }

    /// The button Enter presses from anywhere, if the form declares one.
    fn default_button(&self) -> Option<usize> {
        self.fields
            .iter()
            .position(|f| matches!(f.kind, FieldKind::Button { default: true, .. }))
    }

    /// Press the button at `idx`, whatever its role.
    fn press_button(&mut self, idx: usize) -> FormOutcome<A> {
        let Some(FieldKind::Button { role, action, .. }) = self.fields.get(idx).map(|f| &f.kind)
        else {
            return FormOutcome::nothing();
        };
        let role = *role;
        let mut actions = Vec::new();
        if let Some(a) = action.clone() {
            actions.push(a);
        }
        match role {
            // Accept keeps what the user changed, so nothing is replayed.
            ButtonRole::Accept => FormOutcome::closing(actions),
            ButtonRole::Reject => {
                actions.extend(self.cancel_actions());
                FormOutcome::closing(actions)
            }
            // Apply, Help, Reset: say something and stay put.
            ButtonRole::Stay => FormOutcome {
                actions,
                close: false,
            },
        }
    }

    /// Press the default button, or do nothing if the form has none.
    fn press_default(&mut self) -> FormOutcome<A> {
        match self.default_button() {
            Some(i) => self.press_button(i),
            None => FormOutcome::nothing(),
        }
    }

    /// Commit whatever the focused entry field holds, if it is one.
    ///
    /// Returns the actions to emit, and marks the field dirty when the value
    /// genuinely changed. A number whose buffer will not parse commits
    /// nothing and stays clean: Enter on an untouched field must not enrol it
    /// in the Cancel restore set.
    fn commit_entry(&mut self) -> Vec<A> {
        let focus = self.focus;
        let mut changed = false;
        let actions = match self.fields.get_mut(focus).map(|f| &mut f.kind) {
            Some(FieldKind::Text {
                buffer,
                selected,
                commit,
                ..
            }) => {
                // Simpler than a number: nothing to parse, nothing to clamp,
                // and an empty buffer is a legitimate value rather than
                // "nothing was typed".
                *selected = false;
                changed = true;
                alloc::vec![commit(buffer.as_str())]
            }
            Some(FieldKind::Number {
                value,
                buffer,
                cursor,
                selected,
                min,
                max,
                commit,
                ..
            }) => {
                let Ok(parsed) = buffer.parse::<u32>() else {
                    return Vec::new();
                };
                let clamped = parsed.clamp(*min, *max);
                // Normalise the live buffer to the clamped value either way.
                *buffer = alloc::format!("{clamped}");
                *cursor = buffer.chars().count();
                *selected = false;
                if clamped == *value {
                    Vec::new()
                } else {
                    *value = clamped;
                    changed = true;
                    alloc::vec![commit(clamped)]
                }
            }
            _ => Vec::new(),
        };
        if changed {
            self.mark_dirty(focus);
        }
        actions
    }

    /// What Enter does.
    ///
    /// Enter operates the focused control where there is something to
    /// operate — press a button, open a dropdown, flip a toggle — and
    /// otherwise means "I am finished": it commits the entry field and
    /// presses the default button.
    ///
    /// Opening a dropdown with Enter follows the BIOS-setup tradition this
    /// toolkit descends from, where highlighting a setting and pressing Enter
    /// gives you its value list. Windows dialogs reserve Enter for the
    /// default button and open combo boxes with Alt+Down instead; both are
    /// coherent, and this one suits a text-mode toolkit better.
    ///
    /// [`Self::on_space`] is the uniform partner: Space always operates the
    /// focused control, so nothing depends on remembering which kinds Enter
    /// treats specially.
    fn activate(&mut self) -> FormOutcome<A> {
        let focus = self.focus;
        // A focused button is pressed under every policy: if you tabbed to
        // Cancel, Enter presses Cancel rather than reaching past it.
        if matches!(
            self.fields.get(focus).map(|f| &f.kind),
            Some(FieldKind::Button { .. })
        ) {
            return self.press_button(focus);
        }
        if self.enter_reach == EnterReach::AlwaysAccept {
            let committed = self.commit_entry();
            let mut outcome = self.press_default();
            let mut actions = committed;
            actions.append(&mut outcome.actions);
            outcome.actions = actions;
            return outcome;
        }
        match self.fields.get(focus).map(|f| &f.kind) {
            Some(FieldKind::Choice { selected, .. }) => {
                self.popup = Some(Popup {
                    field: focus,
                    highlight: selected.unwrap_or(0),
                });
                // Opening changes nothing yet; the Enter that picks an option
                // is what marks the field dirty.
                FormOutcome::nothing()
            }
            Some(FieldKind::Button { .. }) => self.press_button(focus),
            // A toggle has something to operate, so Enter operates it rather
            // than reaching past it to the default button.
            Some(FieldKind::Toggle { .. }) => self.flip_toggle(),
            Some(FieldKind::Text { .. }) | Some(FieldKind::Number { .. }) => {
                let committed = self.commit_entry();
                if self.enter_reach == EnterReach::OperateOnly {
                    // Commit the value, but stop there: this form does not
                    // expect Enter to close it.
                    return FormOutcome {
                        actions: committed,
                        close: false,
                    };
                }
                let mut outcome = self.press_default();
                // The field's own value is reported before whatever the
                // button had to say about it.
                let mut actions = committed;
                actions.append(&mut outcome.actions);
                outcome.actions = actions;
                outcome
            }
            _ if self.enter_reach == EnterReach::OperateOnly => FormOutcome::nothing(),
            _ => self.press_default(),
        }
    }

    /// What Space does: act on whatever is focused.
    ///
    /// The uniform partner to Enter. Space always means "operate this
    /// control" — flip the toggle, open the dropdown, press the button —
    /// while Enter additionally means "I am finished" on the fields where
    /// there is nothing to operate. Overlapping is deliberate: Windows has
    /// Space and Enter both pressing a focused button, and having one key
    /// that always works keeps the other free to mean accept.
    ///
    /// A text field is the exception, since there Space has to mean a space.
    fn on_space(&mut self) -> FormOutcome<A> {
        let focus = self.focus;
        match self.fields.get(focus).map(|f| &f.kind) {
            Some(FieldKind::Toggle { .. }) => self.flip_toggle(),
            Some(FieldKind::Choice { selected, .. }) => {
                self.popup = Some(Popup {
                    field: focus,
                    highlight: selected.unwrap_or(0),
                });
                FormOutcome::nothing()
            }
            Some(FieldKind::Button { .. }) => self.press_button(focus),
            // Text takes a literal space; Number ignores it.
            _ => {
                self.type_char(' ');
                FormOutcome::nothing()
            }
        }
    }

    /// Flip the focused toggle and report the action it stands for.
    fn flip_toggle(&mut self) -> FormOutcome<A> {
        let focus = self.focus;
        if let Some(FieldKind::Toggle {
            on,
            on_action,
            off_action,
        }) = self.fields.get_mut(focus).map(|f| &mut f.kind)
        {
            *on = !*on;
            let a = if *on {
                on_action.clone()
            } else {
                off_action.clone()
            };
            self.mark_dirty(focus);
            return FormOutcome::action(a);
        }
        FormOutcome::nothing()
    }

    fn handle_popup(&mut self, ev: FormEvent) -> FormOutcome<A> {
        let Some(popup) = self.popup else {
            return FormOutcome::nothing();
        };
        let count = match self.fields.get(popup.field).map(|f| &f.kind) {
            Some(FieldKind::Choice { options, .. }) => options.len(),
            _ => 0,
        };
        if count == 0 {
            self.popup = None;
            return FormOutcome::nothing();
        }
        match ev {
            FormEvent::Up => {
                let h = (popup.highlight + count - 1) % count;
                self.popup = Some(Popup {
                    highlight: h,
                    ..popup
                });
                FormOutcome::nothing()
            }
            FormEvent::Down => {
                let h = (popup.highlight + 1) % count;
                self.popup = Some(Popup {
                    highlight: h,
                    ..popup
                });
                FormOutcome::nothing()
            }
            FormEvent::Escape => {
                self.popup = None;
                FormOutcome::nothing()
            }
            FormEvent::Enter => {
                self.popup = None;
                let picked = match self.fields.get_mut(popup.field).map(|f| &mut f.kind) {
                    Some(FieldKind::Choice { options, selected }) => {
                        *selected = Some(popup.highlight);
                        options.get(popup.highlight).map(|o| o.action.clone())
                    }
                    _ => None,
                };
                match picked {
                    Some(a) => {
                        // Re-picking the already-selected option counts as a
                        // change. Comparing values to detect a genuine edit
                        // would need `A: PartialEq`, and restoring a field to
                        // the value it already holds is harmless.
                        self.mark_dirty(popup.field);
                        FormOutcome::action(a)
                    }
                    None => FormOutcome::nothing(),
                }
            }
            // A popup list is navigated only with Up/Down/Enter/Escape; the
            // horizontal and Tab movements have no meaning inside it, and an
            // accelerator aimed at a field behind the popup must not reach
            // past it while it is modal.
            FormEvent::Hotkey(_)
            | FormEvent::Char(_)
            | FormEvent::Backspace
            | FormEvent::Left
            | FormEvent::Right
            | FormEvent::Tab
            | FormEvent::BackTab
            | FormEvent::Home
            | FormEvent::End
            | FormEvent::Delete
            | FormEvent::Insert => FormOutcome::nothing(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloc::string::ToString;

    /// A dummy action type, declared locally on purpose. If the model ever
    /// grows a dependency on some consuming application's action enum, these
    /// tests stop compiling — which is the point.
    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    enum TestOp {
        SetColor(u8),
        On,
        Off,
        Period(u32),
        /// Used only by the Number edit-state tests below, which don't care
        /// which action `commit` returns — just that one was returned.
        A,
    }

    fn period(v: u32) -> TestOp {
        TestOp::Period(v)
    }

    fn choice(label: &str, n: u8) -> ChoiceOption<TestOp> {
        ChoiceOption {
            label: label.to_string(),
            action: TestOp::SetColor(n),
        }
    }

    fn fields() -> Vec<Field<TestOp>> {
        alloc::vec![
            Field {
                label: "Colour",
                kind: FieldKind::Choice {
                    options: alloc::vec![choice("Red", 1), choice("Green", 2), choice("Blue", 3)],
                    selected: Some(1),
                },
                restore: alloc::vec![TestOp::SetColor(2)],
            },
            Field {
                label: "Strategy",
                kind: FieldKind::ReadOnly("Fixed".to_string()),
                restore: Vec::new(),
            },
            Field {
                label: "Cycling",
                kind: FieldKind::Toggle {
                    on: false,
                    on_action: TestOp::On,
                    off_action: TestOp::Off,
                },
                restore: alloc::vec![TestOp::Off],
            },
            Field {
                label: "Period",
                kind: FieldKind::Number {
                    value: 120,
                    buffer: String::new(),
                    cursor: 0,
                    selected: false,
                    overtype: false,
                    min: 10,
                    max: 3600,
                    unit: "s",
                    commit: period,
                },
                restore: alloc::vec![TestOp::Period(120)],
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
        ]
    }

    fn form() -> FormState<TestOp> {
        FormState::new("TEST", fields())
    }

    #[test]
    fn focus_starts_on_the_first_focusable_field() {
        assert_eq!(form().focus(), 0);
    }

    #[test]
    fn a_form_with_no_focusable_field_is_inert_rather_than_focusing_readonly() {
        use alloc::string::ToString;
        let f: FormState<TestOp> = FormState::new(
            "DEGENERATE",
            alloc::vec![
                Field {
                    label: "A",
                    kind: FieldKind::ReadOnly("x".to_string()),
                    restore: Vec::new(),
                },
                Field {
                    label: "B",
                    kind: FieldKind::ReadOnly("y".to_string()),
                    restore: Vec::new(),
                },
            ],
        );
        // Focus is meaningless here, but it must not claim a ReadOnly field is
        // focusable, and nothing may panic or loop.
        let mut f = f;
        assert!(f.handle(FormEvent::Down).actions.is_empty());
        assert!(f.handle(FormEvent::Up).actions.is_empty());
        assert!(f.handle(FormEvent::Enter).actions.is_empty());
        assert!(!f.handle(FormEvent::Enter).close);
    }

    #[test]
    fn focus_skips_readonly_fields_in_both_directions() {
        let mut f = form();
        f.handle(FormEvent::Down);
        assert_eq!(f.focus(), 2, "index 1 is ReadOnly and must be skipped");
        f.handle(FormEvent::Up);
        assert_eq!(f.focus(), 0);
    }

    #[test]
    fn up_down_wrap_by_row_landing_on_the_button_bar_at_column_zero() {
        // Rows: [0], [2], [3], [4,5]. Up from row 0 wraps to the last row (the
        // button bar) at the clamped column 0 = OK (field 4), not Cancel.
        let mut f = form();
        f.handle(FormEvent::Up);
        assert_eq!(f.focus(), 4, "Up from the top wraps to OK, the bar's first");
        f.handle(FormEvent::Down);
        assert_eq!(f.focus(), 0, "Down from the bar wraps to the top field");
    }

    #[test]
    fn left_right_move_within_the_button_bar_only() {
        // Left/Right move between OK and Cancel (one row); on a single-field row
        // they are a no-op.
        let mut f = form();
        f.set_focus(4);
        f.handle(FormEvent::Right);
        assert_eq!(f.focus(), 5, "Right: OK -> Cancel");
        f.handle(FormEvent::Left);
        assert_eq!(f.focus(), 4, "Left: Cancel -> OK");

        f.set_focus(0); // a single-field row
        f.handle(FormEvent::Right);
        assert_eq!(f.focus(), 0, "Left/Right do nothing on a single-field row");
    }

    #[test]
    fn up_down_never_move_between_the_two_side_by_side_buttons() {
        // The authentic CUA rule: Up/Down change rows, so from OK they leave the
        // button bar entirely rather than stepping to Cancel.
        let mut f = form();
        f.set_focus(4); // OK
        f.handle(FormEvent::Up);
        assert_eq!(f.focus(), 3, "Up from OK leaves the bar to the field above");
        f.set_focus(4);
        f.handle(FormEvent::Down);
        assert_ne!(f.focus(), 5, "Down from OK must not land on Cancel");
    }

    #[test]
    fn tab_and_back_tab_step_linearly_through_every_focusable_field() {
        // Tab is the primary control key: it visits both buttons in order,
        // unlike Up/Down which treat the bar as one row.
        let mut f = form();
        let order = [2usize, 3, 4, 5, 0]; // from 0, forward, skipping ReadOnly 1
        for expected in order {
            f.handle(FormEvent::Tab);
            assert_eq!(f.focus(), expected);
        }
        // Shift+Tab reverses.
        f.handle(FormEvent::BackTab);
        assert_eq!(f.focus(), 5);
    }

    #[test]
    fn enter_on_a_choice_opens_a_popup_highlighting_the_current_value() {
        let mut f = form();
        let out = f.handle(FormEvent::Enter);
        assert!(out.actions.is_empty());
        assert!(!out.close);
        let p = f.popup().expect("popup open");
        assert_eq!(p.field, 0);
        assert_eq!(p.highlight, 1, "highlight starts on the selected option");
    }

    #[test]
    fn popup_selection_emits_that_options_action_and_closes_the_popup() {
        let mut f = form();
        f.handle(FormEvent::Enter);
        f.handle(FormEvent::Down);
        let out = f.handle(FormEvent::Enter);
        assert_eq!(out.actions, alloc::vec![TestOp::SetColor(3)]);
        assert!(!out.close, "picking an option must not close the form");
        assert!(f.popup().is_none());
    }

    #[test]
    fn escape_closes_the_popup_not_the_form() {
        let mut f = form();
        f.handle(FormEvent::Enter);
        let out = f.handle(FormEvent::Escape);
        assert!(out.actions.is_empty());
        assert!(
            !out.close,
            "Escape with a popup open must not close the form"
        );
        assert!(f.popup().is_none());
    }

    #[test]
    fn toggle_flips_and_emits_the_matching_action() {
        let mut f = form();
        f.set_focus(2);
        let out = f.handle(FormEvent::Enter);
        assert_eq!(out.actions, alloc::vec![TestOp::On]);
        let out = f.handle(FormEvent::Enter);
        assert_eq!(out.actions, alloc::vec![TestOp::Off]);
    }

    #[test]
    fn number_accepts_digits_and_commits_on_enter() {
        let mut f = form();
        f.set_focus(3);
        f.handle(FormEvent::Char('2'));
        f.handle(FormEvent::Char('4'));
        f.handle(FormEvent::Char('0'));
        let out = f.handle(FormEvent::Enter);
        assert_eq!(out.actions, alloc::vec![TestOp::Period(240)]);
    }

    #[test]
    fn number_backspace_pops_and_non_digits_are_ignored() {
        let mut f = form();
        f.set_focus(3);
        f.handle(FormEvent::Char('9'));
        f.handle(FormEvent::Char('x'));
        f.handle(FormEvent::Char('9'));
        f.handle(FormEvent::Backspace);
        let out = f.handle(FormEvent::Enter);
        assert_eq!(
            out.actions,
            alloc::vec![TestOp::Period(10)],
            "'9' clamps up to min"
        );
    }

    #[test]
    fn number_caps_the_buffer_at_four_characters() {
        let mut f = form();
        f.set_focus(3);
        // Five digits typed. With the 4-char cap the buffer is "1234" -> 1234.
        // Without it the buffer would be "12345", which clamps to max (3600),
        // so this assertion actually discriminates.
        for c in ['1', '2', '3', '4', '5'] {
            f.handle(FormEvent::Char(c));
        }
        let out = f.handle(FormEvent::Enter);
        assert_eq!(out.actions, alloc::vec![TestOp::Period(1234)]);
    }

    #[test]
    fn number_clamps_a_value_above_max() {
        let mut f = form();
        f.set_focus(3);
        for _ in 0..4 {
            f.handle(FormEvent::Char('9'));
        }
        let out = f.handle(FormEvent::Enter);
        assert_eq!(
            out.actions,
            alloc::vec![TestOp::Period(3600)],
            "9999 clamps to max"
        );
    }

    #[test]
    fn number_with_an_empty_buffer_commits_nothing_but_still_accepts() {
        let mut f = form().with_enter_reach(EnterReach::AcceptWhenIdle);
        f.set_focus(3);
        let out = f.handle(FormEvent::Enter);
        // Nothing was typed, so nothing is committed — but Enter still means
        // "I am finished", so it reaches the default button regardless.
        assert!(out.actions.is_empty());
        assert!(out.close);
    }

    #[test]
    fn a_seeded_number_field_replaces_its_value_on_the_first_digit() {
        // Buffer pre-seeded with "120" (as the settings/timing builders do).
        let mut f = FormState::new(
            "T",
            alloc::vec![
                Field {
                    label: "Period",
                    kind: FieldKind::Number {
                        value: 120,
                        buffer: "120".to_string(),
                        cursor: 3,
                        selected: true,
                        overtype: false,
                        min: 10,
                        max: 3600,
                        unit: "s",
                        commit: period,
                    },
                    restore: alloc::vec![TestOp::Period(120)],
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
        f.set_focus(0);
        for c in ['2', '4', '0'] {
            f.handle(FormEvent::Char(c));
        }
        let out = f.handle(FormEvent::Enter);
        assert_eq!(
            out.actions,
            alloc::vec![TestOp::Period(240)],
            "the seed is replaced by the typed value, not appended to"
        );
    }

    #[test]
    fn a_seeded_number_field_clears_on_first_backspace() {
        let mut f = FormState::new(
            "T",
            alloc::vec![
                Field {
                    label: "Period",
                    kind: FieldKind::Number {
                        value: 120,
                        buffer: "120".to_string(),
                        cursor: 3,
                        selected: true,
                        overtype: false,
                        min: 10,
                        max: 3600,
                        unit: "s",
                        commit: period,
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
        f.set_focus(0);
        f.handle(FormEvent::Backspace);
        // Seed cleared; buffer empty; Enter now emits nothing.
        let out = f.handle(FormEvent::Enter);
        assert!(
            out.actions.is_empty(),
            "backspace cleared the seed, then empty commits nothing"
        );
    }

    #[test]
    fn ok_closes_without_emitting_anything() {
        let mut f = form();
        f.set_focus(4);
        let out = f.handle(FormEvent::Enter);
        assert!(out.actions.is_empty());
        assert!(out.close);
    }

    #[test]
    fn cancel_without_edits_emits_nothing() {
        let mut f = form();
        f.set_focus(5);
        let out = f.handle(FormEvent::Enter);
        assert!(out.close);
        assert!(
            out.actions.is_empty(),
            "an untouched field must not be 'restored' — its action may have \
             side effects beyond its own field"
        );
    }

    #[test]
    fn escape_with_no_popup_cancels_the_form() {
        let mut f = form();
        let out = f.handle(FormEvent::Escape);
        assert!(out.close);
        assert!(out.actions.is_empty());
    }

    #[test]
    fn cancel_restores_only_the_fields_the_user_changed() {
        let mut f = form();
        // Change the colour to Blue. Cycling and Period are left alone.
        f.handle(FormEvent::Enter);
        f.handle(FormEvent::Down);
        f.handle(FormEvent::Enter);
        // Cancel.
        f.set_focus(5);
        let out = f.handle(FormEvent::Enter);
        assert_eq!(
            out.actions,
            alloc::vec![TestOp::SetColor(2)],
            "only the edited field is restored, and to its opening value"
        );
    }

    #[test]
    fn cancel_restores_several_edited_fields_in_field_order() {
        let mut f = form();
        // Flip cycling on first...
        f.set_focus(2);
        f.handle(FormEvent::Enter);
        // ...then change the colour to Blue.
        f.set_focus(0);
        f.handle(FormEvent::Enter);
        f.handle(FormEvent::Down);
        f.handle(FormEvent::Enter);
        let out = f.handle(FormEvent::Escape);
        assert_eq!(
            out.actions,
            alloc::vec![TestOp::SetColor(2), TestOp::Off],
            "field order, not edit order"
        );
    }

    #[test]
    fn a_number_field_that_commits_nothing_is_not_dirtied() {
        let mut f = form();
        f.set_focus(3);
        // Enter on an empty buffer: no action, and no enrolment in Cancel.
        f.handle(FormEvent::Enter);
        let out = f.handle(FormEvent::Escape);
        assert!(out.actions.is_empty());
    }

    #[test]
    fn a_committed_number_field_is_restored_by_cancel() {
        let mut f = form();
        f.set_focus(3);
        f.handle(FormEvent::Char('3'));
        f.handle(FormEvent::Char('0'));
        f.handle(FormEvent::Enter);
        let out = f.handle(FormEvent::Escape);
        assert_eq!(out.actions, alloc::vec![TestOp::Period(120)]);
    }

    #[test]
    fn committing_a_number_accepts_the_form_in_one_keystroke() {
        let mut f = form().with_enter_reach(EnterReach::AcceptWhenIdle);
        f.set_focus(3);
        f.handle(FormEvent::Char('3'));
        f.handle(FormEvent::Char('0'));
        let outcome = f.handle(FormEvent::Enter);
        // Type a value, press Enter, done — the entry field commits and then
        // lets Enter through to the default button rather than swallowing it.
        assert!(
            outcome.close,
            "Enter on an entry field presses the default button"
        );
        assert!(
            !outcome.actions.is_empty(),
            "and the typed value is reported before the form closes"
        );
    }

    #[test]
    fn refresh_fields_replaces_values_but_keeps_focus_popup_and_dirt() {
        let mut f = form();
        // Dirty the colour field and open a popup on it.
        f.handle(FormEvent::Enter);
        f.handle(FormEvent::Down);
        f.handle(FormEvent::Enter);
        f.set_focus(2);
        f.handle(FormEvent::Enter); // popup-free toggle, marks field 2 dirty
        f.set_focus(0);
        f.handle(FormEvent::Enter); // popup open on field 0

        // A rebuild whose colour field reports a different current value.
        let mut rebuilt = fields();
        rebuilt[0].kind = FieldKind::Choice {
            options: alloc::vec![choice("Red", 1), choice("Green", 2), choice("Blue", 3)],
            selected: Some(2),
        };
        rebuilt[0].restore = alloc::vec![TestOp::SetColor(3)];
        f.refresh_fields(rebuilt);

        assert_eq!(f.focus(), 0, "focus survives");
        assert!(f.popup().is_some(), "an open popup survives");
        match &f.fields()[0].kind {
            FieldKind::Choice { selected, .. } => assert_eq!(*selected, Some(2), "values refresh"),
            _ => panic!("not a Choice"),
        }
        f.handle(FormEvent::Escape); // close the popup
        let out = f.handle(FormEvent::Escape);
        assert_eq!(
            out.actions,
            alloc::vec![TestOp::SetColor(2), TestOp::Off],
            "originals are the OPENING values, not the refreshed ones"
        );
    }

    #[test]
    fn refresh_fields_with_a_different_length_does_not_panic() {
        let mut f = form();
        f.set_focus(2);
        f.handle(FormEvent::Enter);
        f.refresh_fields(alloc::vec![Field {
            label: "Only",
            kind: FieldKind::ReadOnly("x".to_string()),
            restore: Vec::new(),
        }]);
        // Focus is now past the end; nothing may panic.
        let _ = f.handle(FormEvent::Enter);
        let out = f.handle(FormEvent::Escape);
        assert!(out.close);
        assert_eq!(
            out.actions,
            alloc::vec![TestOp::Off],
            "the old originals survive a length change"
        );
    }

    #[test]
    fn a_field_with_no_restore_action_contributes_nothing_to_cancel() {
        use alloc::string::ToString;
        let mut f = FormState::new(
            "T",
            alloc::vec![
                Field {
                    label: "Unset",
                    kind: FieldKind::Choice {
                        options: alloc::vec![
                            ChoiceOption {
                                label: "A".to_string(),
                                action: TestOp::On
                            },
                            ChoiceOption {
                                label: "B".to_string(),
                                action: TestOp::Off
                            },
                        ],
                        selected: None,
                    },
                    // Nothing was selected when the form opened, so there is
                    // no opening value to go back to.
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
        );
        // Genuinely edit it, so only the missing `restore` can suppress the action.
        f.handle(FormEvent::Enter);
        f.handle(FormEvent::Enter);
        let out = f.handle(FormEvent::Escape);
        assert!(out.close);
        assert!(
            out.actions.is_empty(),
            "Cancel must not invent an action for a field with no opening value"
        );
    }

    /// A form holding one Text field followed by an OK button.
    #[test]
    fn parse_mnemonic_strips_markers_and_reports_the_letter() {
        assert_eq!(parse_mnemonic("~O~pen"), ("Open".into(), Some(('O', 0))));
        assert_eq!(parse_mnemonic("Sa~v~e"), ("Save".into(), Some(('v', 2))));
        assert_eq!(parse_mnemonic("Plain"), ("Plain".into(), None));
    }

    #[test]
    fn only_the_first_marked_letter_claims_the_accelerator() {
        // A label claiming two is a mistake; taking the first is predictable.
        assert_eq!(parse_mnemonic("~A~b~C~"), ("AbC".into(), Some(('A', 0))));
    }

    #[test]
    fn a_marker_never_occupies_a_cell() {
        let (text, _) = parse_mnemonic("~O~pen");
        assert_eq!(text.chars().count(), 4, "tildes must not be drawn");
    }

    #[test]
    fn space_presses_a_focused_button() {
        let mut f = hotkey_form();
        f.set_focus(2); // OK
        let outcome = f.handle(FormEvent::Char(' '));
        assert!(outcome.close, "Space operates whatever is focused");
    }

    #[test]
    fn space_opens_a_dropdown_just_as_enter_does() {
        let mut f = form();
        f.handle(FormEvent::Char(' '));
        assert!(f.popup().is_some(), "Space is the uniform operate key");
    }

    #[test]
    fn enter_opens_a_dropdown_too_which_is_the_bios_setup_habit() {
        let mut f = form();
        f.handle(FormEvent::Enter);
        assert!(f.popup().is_some());
    }

    #[test]
    fn space_flips_a_toggle() {
        let mut f = hotkey_form();
        let outcome = f.handle(FormEvent::Char(' '));
        assert_eq!(outcome.actions, alloc::vec![TestOp::On]);
        assert!(!outcome.close);
    }

    #[test]
    fn space_in_a_text_field_types_a_space_rather_than_operating_anything() {
        let mut f = text_form("", 16);
        f.handle(FormEvent::Char('a'));
        f.handle(FormEvent::Char(' '));
        f.handle(FormEvent::Char('b'));
        assert_eq!(text_of(&f).0, "a b");
    }

    #[test]
    fn enter_reaches_the_default_button_from_a_toggle_only_after_it_flips() {
        // A toggle has something to operate, so Enter operates it rather than
        // reaching past it.
        let mut f = hotkey_form();
        let outcome = f.handle(FormEvent::Enter);
        assert_eq!(outcome.actions, alloc::vec![TestOp::On]);
        assert!(!outcome.close);
    }

    #[test]
    fn a_form_without_a_default_button_simply_does_not_accept_on_enter() {
        let mut f = text_form("hi", 16);
        // text_form's second field is a plain OK built by Field::ok(), which
        // is default; strip that to prove the fallback.
        let mut fields = alloc::vec![];
        for (i, fld) in f.fields().iter().enumerate() {
            if i == 0 {
                fields.push(fld.clone());
            }
        }
        let mut bare = FormState::new("T", fields);
        let outcome = bare.handle(FormEvent::Enter);
        assert!(!outcome.close, "nothing to press, so nothing happens");
        let _ = f.handle(FormEvent::Escape);
    }

    #[test]
    fn enter_does_not_close_a_form_by_default() {
        // OperateOnly is the default: a form that does not expect Enter to
        // close it never will, whatever its buttons say.
        let mut f = text_form("hi", 16);
        assert_eq!(f.enter_reach(), EnterReach::OperateOnly);
        let outcome = f.handle(FormEvent::Enter);
        assert!(!outcome.close, "Enter commits but does not accept");
        assert_eq!(
            outcome.actions,
            alloc::vec![TestOp::A],
            "the value is still committed"
        );
    }

    #[test]
    fn always_accept_reaches_the_default_button_even_from_a_dropdown() {
        let mut f = form().with_enter_reach(EnterReach::AlwaysAccept);
        let outcome = f.handle(FormEvent::Enter);
        assert!(
            f.popup().is_none(),
            "under AlwaysAccept nothing else consumes Enter"
        );
        assert!(outcome.close);
    }

    #[test]
    fn always_accept_still_presses_a_focused_button_rather_than_the_default() {
        let mut f = role_form().with_enter_reach(EnterReach::AlwaysAccept);
        f.set_focus(3); // Cancel
        let outcome = f.handle(FormEvent::Enter);
        assert!(outcome.close);
        assert!(
            outcome.actions.contains(&TestOp::Off) || outcome.actions.is_empty(),
            "Cancel was pressed, not OK"
        );
    }

    #[test]
    fn accept_when_idle_leaves_the_dropdown_its_enter() {
        let mut f = form().with_enter_reach(EnterReach::AcceptWhenIdle);
        f.handle(FormEvent::Enter);
        assert!(f.popup().is_some(), "a dropdown still opens on Enter");
    }

    #[test]
    fn space_operates_the_control_under_every_policy() {
        for reach in [
            EnterReach::OperateOnly,
            EnterReach::AcceptWhenIdle,
            EnterReach::AlwaysAccept,
        ] {
            let mut f = form().with_enter_reach(reach);
            f.handle(FormEvent::Char(' '));
            assert!(f.popup().is_some(), "Space is uniform across {reach:?}");
        }
    }

    fn role_form() -> FormState<TestOp> {
        FormState::new(
            "T",
            alloc::vec![
                Field {
                    label: "T",
                    kind: FieldKind::Toggle {
                        on: false,
                        on_action: TestOp::On,
                        off_action: TestOp::Off,
                    },
                    restore: alloc::vec![TestOp::Off],
                },
                Field::button("~A~pply", ButtonRole::Stay, Some(TestOp::A)),
                Field::ok(),
                Field::cancel(),
            ],
        )
    }

    #[test]
    fn a_stay_button_emits_its_action_without_closing() {
        let mut f = role_form();
        let outcome = f.handle(FormEvent::Hotkey('a'));
        assert_eq!(outcome.actions, alloc::vec![TestOp::A]);
        assert!(!outcome.close, "Apply stays put");
    }

    #[test]
    fn an_accept_button_closes_without_replaying_anything() {
        let mut f = role_form();
        f.handle(FormEvent::Enter); // flip the toggle, making it dirty
        let outcome = f.handle(FormEvent::Hotkey('o'));
        assert!(outcome.close);
        assert!(
            !outcome.actions.contains(&TestOp::Off),
            "Accept keeps the change rather than restoring it"
        );
    }

    #[test]
    fn a_reject_button_closes_and_restores_what_was_touched() {
        let mut f = role_form();
        f.handle(FormEvent::Enter); // flip the toggle, making it dirty
        let outcome = f.handle(FormEvent::Hotkey('c'));
        assert!(outcome.close);
        assert!(
            outcome.actions.contains(&TestOp::Off),
            "Reject replays the restore action of the field that changed"
        );
    }

    #[test]
    fn a_buttons_own_action_is_emitted_before_any_restore() {
        let mut f = FormState::new(
            "T",
            alloc::vec![
                Field {
                    label: "T",
                    kind: FieldKind::Toggle {
                        on: false,
                        on_action: TestOp::On,
                        off_action: TestOp::Off,
                    },
                    restore: alloc::vec![TestOp::Off],
                },
                Field::button("~D~iscard", ButtonRole::Reject, Some(TestOp::A)),
            ],
        );
        f.handle(FormEvent::Enter); // dirty the toggle
        let outcome = f.handle(FormEvent::Hotkey('d'));
        assert_eq!(
            outcome.actions.first(),
            Some(&TestOp::A),
            "the button speaks first, then the restores follow"
        );
        assert!(outcome.actions.contains(&TestOp::Off));
    }

    #[test]
    fn a_custom_button_claims_the_accelerator_marked_in_its_own_label() {
        let mut f = role_form();
        // 'p' is not marked; ~A~pply claims 'a'.
        let outcome = f.handle(FormEvent::Hotkey('p'));
        assert!(outcome.actions.is_empty());
        assert!(!outcome.close);
    }

    fn hotkey_form() -> FormState<TestOp> {
        FormState::new(
            "T",
            alloc::vec![
                Field {
                    label: "~S~ound",
                    kind: FieldKind::Toggle {
                        on: false,
                        on_action: TestOp::On,
                        off_action: TestOp::Off,
                    },
                    restore: alloc::vec![TestOp::Off],
                },
                Field {
                    label: "Unmarked",
                    kind: FieldKind::Toggle {
                        on: false,
                        on_action: TestOp::On,
                        off_action: TestOp::Off,
                    },
                    restore: alloc::vec![TestOp::Off],
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
                Field {
                    label: "",
                    kind: FieldKind::Button {
                        label: "~C~ancel",
                        role: ButtonRole::Reject,
                        action: None,
                        default: false
                    },
                    restore: alloc::vec![],
                },
            ],
        )
    }

    #[test]
    fn a_hotkey_moves_focus_to_the_field_that_claimed_it() {
        let mut f = hotkey_form();
        f.set_focus(1);
        let outcome = f.handle(FormEvent::Hotkey('s'));
        assert_eq!(f.focus(), 0, "focus moved to the ~S~ound field");
        // Focusing is all it does — a non-button must not be activated.
        assert!(outcome.actions.is_empty());
    }

    #[test]
    fn a_hotkey_matches_regardless_of_case() {
        let mut f = hotkey_form();
        f.set_focus(1);
        f.handle(FormEvent::Hotkey('S'));
        assert_eq!(f.focus(), 0);
    }

    #[test]
    fn a_hotkey_on_a_button_presses_it_rather_than_just_focusing_it() {
        let mut f = hotkey_form();
        let outcome = f.handle(FormEvent::Hotkey('o')); // OK
        assert_eq!(f.focus(), 2);
        assert!(outcome.close, "OK closes the form");
    }

    #[test]
    fn the_cancel_accelerator_closes_and_restores() {
        let mut f = hotkey_form();
        let outcome = f.handle(FormEvent::Hotkey('c'));
        assert_eq!(f.focus(), 3);
        assert!(outcome.close);
    }

    #[test]
    fn an_unclaimed_hotkey_changes_nothing() {
        let mut f = hotkey_form();
        f.set_focus(1);
        let outcome = f.handle(FormEvent::Hotkey('z'));
        assert_eq!(f.focus(), 1);
        assert!(outcome.actions.is_empty());
        assert!(!outcome.close);
    }

    #[test]
    fn a_hotkey_does_not_reach_past_an_open_popup() {
        // The popup is modal; an accelerator aimed at a field behind it must
        // not fire while it is up.
        let mut f = form(); // field 0 is a Choice
        f.handle(FormEvent::Enter); // open the popup
        assert!(f.popup().is_some());
        let outcome = f.handle(FormEvent::Hotkey('o'));
        assert!(f.popup().is_some(), "popup stays open");
        assert!(!outcome.close, "OK must not have been pressed behind it");
    }

    fn text_form(initial: &str, max_len: usize) -> FormState<TestOp> {
        FormState::new(
            "T",
            alloc::vec![
                Field {
                    label: "Name",
                    kind: FieldKind::Text {
                        buffer: alloc::string::String::from(initial),
                        cursor: initial.chars().count(),
                        selected: true,
                        overtype: false,
                        max_len,
                        commit: |_| TestOp::A,
                    },
                    restore: alloc::vec![TestOp::Off],
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

    /// (buffer, cursor, selected, overtype) of the first field.
    fn text_of(f: &FormState<TestOp>) -> (alloc::string::String, usize, bool, bool) {
        match &f.fields()[0].kind {
            FieldKind::Text {
                buffer,
                cursor,
                selected,
                overtype,
                ..
            } => (buffer.clone(), *cursor, *selected, *overtype),
            _ => panic!("field 0 is not Text"),
        }
    }

    #[test]
    fn text_accepts_letters_where_a_number_field_would_refuse_them() {
        let mut f = text_form("", 16);
        f.handle(FormEvent::Char('L'));
        f.handle(FormEvent::Char('o'));
        assert_eq!(text_of(&f).0, "Lo");
    }

    #[test]
    fn typing_into_a_selected_text_field_replaces_the_whole_value() {
        let mut f = text_form("Loki", 16);
        assert!(text_of(&f).2, "starts whole-selected");
        f.handle(FormEvent::Char('X'));
        let (buffer, cursor, selected, _) = text_of(&f);
        assert_eq!(buffer, "X");
        assert_eq!(cursor, 1);
        assert!(!selected);
    }

    #[test]
    fn text_inserts_at_the_caret_in_insert_mode() {
        let mut f = text_form("ac", 16);
        f.handle(FormEvent::Home);
        f.handle(FormEvent::Right); // caret between 'a' and 'c'
        f.handle(FormEvent::Char('b'));
        assert_eq!(text_of(&f).0, "abc");
    }

    #[test]
    fn text_replaces_at_the_caret_in_overtype_mode() {
        let mut f = text_form("abc", 16);
        f.handle(FormEvent::Insert); // overtype on, deselects
        f.handle(FormEvent::Home);
        f.handle(FormEvent::Char('X'));
        let (buffer, cursor, _, overtype) = text_of(&f);
        assert!(overtype);
        assert_eq!(buffer, "Xbc");
        assert_eq!(cursor, 1);
    }

    #[test]
    fn text_stops_accepting_input_at_max_len() {
        let mut f = text_form("", 3);
        for c in ['a', 'b', 'c', 'd', 'e'] {
            f.handle(FormEvent::Char(c));
        }
        assert_eq!(text_of(&f).0, "abc");
    }

    #[test]
    fn overtype_can_still_replace_once_a_text_field_is_full() {
        let mut f = text_form("abc", 3);
        f.handle(FormEvent::Insert);
        f.handle(FormEvent::Home);
        f.handle(FormEvent::Char('Z'));
        // Replacing does not lengthen the buffer, so max_len must not block it.
        assert_eq!(text_of(&f).0, "Zbc");
    }

    #[test]
    fn backspace_and_delete_remove_on_either_side_of_the_caret() {
        let mut f = text_form("abc", 16);
        f.handle(FormEvent::End);
        f.handle(FormEvent::Backspace);
        assert_eq!(text_of(&f).0, "ab");
        f.handle(FormEvent::Home);
        f.handle(FormEvent::Delete);
        assert_eq!(text_of(&f).0, "b");
    }

    #[test]
    fn the_caret_is_counted_in_characters_not_bytes() {
        // 'é' is two UTF-8 bytes but one cell, so the caret must land at 1.
        let mut f = text_form("", 16);
        f.handle(FormEvent::Char('é'));
        let (buffer, cursor, ..) = text_of(&f);
        assert_eq!(buffer, "é");
        assert_eq!(cursor, 1, "caret counts characters, not bytes");
        // And editing around it must not split the encoding.
        f.handle(FormEvent::Char('x'));
        f.handle(FormEvent::Home);
        f.handle(FormEvent::Delete);
        assert_eq!(text_of(&f).0, "x");
    }

    #[test]
    fn enter_commits_the_text_and_accepts_the_form() {
        let mut f = text_form("hi", 16).with_enter_reach(EnterReach::AcceptWhenIdle);
        let outcome = f.handle(FormEvent::Enter);
        assert_eq!(outcome.actions, alloc::vec![TestOp::A]);
        assert!(
            outcome.close,
            "the text commits, then Enter reaches the default button"
        );
    }

    #[test]
    fn an_emptied_text_field_still_commits_because_empty_is_a_value() {
        // Unlike Number, where an unparseable buffer means "nothing typed",
        // empty text is a legitimate value the caller may want.
        let mut f = text_form("abc", 16);
        f.handle(FormEvent::Backspace); // whole-selected -> clears
        assert_eq!(text_of(&f).0, "");
        let outcome = f.handle(FormEvent::Enter);
        assert_eq!(outcome.actions, alloc::vec![TestOp::A]);
    }

    #[test]
    fn left_and_right_move_the_caret_inside_a_text_field() {
        let mut f = text_form("abc", 16);
        f.handle(FormEvent::Left); // deselect, caret to 0
        assert_eq!(text_of(&f).1, 0);
        f.handle(FormEvent::Right);
        assert_eq!(text_of(&f).1, 1);
        // ...and never past either end.
        for _ in 0..10 {
            f.handle(FormEvent::Right);
        }
        assert_eq!(text_of(&f).1, 3);
    }

    #[test]
    fn re_entering_a_text_field_reselects_the_whole_value() {
        let mut f = text_form("abc", 16);
        f.handle(FormEvent::Left); // deselect
        assert!(!text_of(&f).2);
        f.handle(FormEvent::Tab); // -> OK
        f.handle(FormEvent::BackTab); // back onto the text field
        assert!(text_of(&f).2, "focus-in reselects, as Turbo Vision did");
    }

    fn num() -> FormState<TestOp> {
        let mut f = FormState::new(
            "T",
            alloc::vec![
                Field {
                    label: "N",
                    kind: FieldKind::Number {
                        value: 120,
                        buffer: alloc::string::String::from("120"),
                        cursor: 3,
                        selected: true,
                        overtype: false,
                        min: 10,
                        max: 3600,
                        unit: "s",
                        commit: |_| TestOp::A,
                    },
                    // Non-empty on purpose: `enter_unchanged_value_does_not_dirty_or_emit`
                    // relies on this to give its Cancel assertion teeth. An
                    // empty `restore` would make `cancel_actions()` return
                    // empty regardless of whether the field was wrongly
                    // marked dirty, so that test would pass even with a bug.
                    restore: alloc::vec![TestOp::A],
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
        f.set_focus(0);
        f
    }
    fn buf(f: &FormState<TestOp>) -> (&str, usize, bool, bool) {
        match &f.fields()[0].kind {
            FieldKind::Number {
                buffer,
                cursor,
                selected,
                overtype,
                ..
            } => (buffer.as_str(), *cursor, *selected, *overtype),
            _ => panic!("field 0 is a Number"),
        }
    }

    #[test]
    fn selected_then_digit_replaces_whole_value() {
        let mut f = num();
        f.handle(FormEvent::Char('5'));
        assert_eq!(buf(&f), ("5", 1, false, false));
    }

    #[test]
    fn arrow_then_digit_edits_in_place() {
        let mut f = num();
        f.handle(FormEvent::Left); // deselect, cursor -> 0
        assert_eq!(buf(&f), ("120", 0, false, false));
        f.handle(FormEvent::Char('9')); // insert at 0
        assert_eq!(buf(&f), ("9120", 1, false, false));
    }

    #[test]
    fn insert_respects_the_four_digit_cap() {
        let mut f = num();
        f.handle(FormEvent::End); // deselect, cursor -> 3
        f.handle(FormEvent::Char('4')); // "1204", cursor 4
        f.handle(FormEvent::Char('5')); // full -> ignored
        assert_eq!(buf(&f), ("1204", 4, false, false));
    }

    #[test]
    fn overtype_replaces_digit_under_cursor() {
        let mut f = num();
        f.handle(FormEvent::Insert); // overtype on, deselect
        assert!(buf(&f).3);
        f.handle(FormEvent::Home); // cursor -> 0
        f.handle(FormEvent::Char('9')); // replace '1' -> "920", cursor 1
        assert_eq!(buf(&f), ("920", 1, false, true));
    }

    #[test]
    fn backspace_and_delete_remove_one_digit() {
        let mut f = num();
        f.handle(FormEvent::End); // cursor 3
        f.handle(FormEvent::Backspace); // "12", cursor 2
        assert_eq!(buf(&f), ("12", 2, false, false));
        f.handle(FormEvent::Home); // cursor 0
        f.handle(FormEvent::Delete); // remove '1' -> "2"
        assert_eq!(buf(&f), ("2", 0, false, false));
    }

    #[test]
    fn selected_backspace_clears_all() {
        let mut f = num();
        f.handle(FormEvent::Backspace);
        assert_eq!(buf(&f), ("", 0, false, false));
    }

    #[test]
    fn enter_unchanged_value_does_not_dirty_or_emit() {
        let mut f = num(); // buffer "120" == value 120
        let out = f.handle(FormEvent::Enter);
        assert!(out.actions.is_empty(), "no action when value is unchanged");
        // Cancel restores nothing because the field was never dirtied.
        let cancel = f.handle(FormEvent::Escape);
        assert!(
            cancel.actions.is_empty(),
            "untouched field not enrolled in Cancel"
        );
    }

    #[test]
    fn enter_on_an_unchanged_value_accepts_without_dirtying_it() {
        // "Open the dialog, press Enter to take the defaults" — Enter must
        // finish the form even when nothing was typed, and must not enrol the
        // untouched field in the Cancel restore set on the way out.
        let mut f = num().with_enter_reach(EnterReach::AcceptWhenIdle);
        let out = f.handle(FormEvent::Enter);
        assert!(out.actions.is_empty(), "unchanged value emits no action");
        assert!(out.close, "Enter still reaches the default button");

        // A fresh form, to check the field stayed clean rather than merely
        // silent: Escape must restore nothing.
        let mut f = num().with_enter_reach(EnterReach::AcceptWhenIdle);
        f.handle(FormEvent::Enter);
        let cancel = f.handle(FormEvent::Escape);
        assert!(
            cancel.actions.is_empty(),
            "the untouched field must not be enrolled in Cancel"
        );
    }

    #[test]
    fn enter_changed_value_clamps_and_emits() {
        let mut f = num();
        f.handle(FormEvent::Char('5')); // buffer "5"
        let out = f.handle(FormEvent::Enter); // 5 clamps to min 10
        assert_eq!(out.actions.len(), 1, "changed value emits a commit");
    }

    #[test]
    fn tabbing_back_onto_a_number_reselects_it() {
        let mut f = num();
        f.handle(FormEvent::Left); // deselect
        assert!(!buf(&f).2);
        f.handle(FormEvent::Tab); // -> OK button
        f.handle(FormEvent::BackTab); // back onto the Number
        assert!(buf(&f).2, "re-entering re-selects the whole value");
    }

    #[test]
    fn overtype_at_end_appends_within_cap() {
        let mut f = num();
        f.handle(FormEvent::Insert); // overtype on, deselect; cursor stays at 3 (already end)
        f.handle(FormEvent::Char('4')); // nothing under the cursor -> append: "1204", cursor 4
        assert_eq!(buf(&f), ("1204", 4, false, true));
        f.handle(FormEvent::Char('5')); // at the 4-digit cap -> ignored
        assert_eq!(buf(&f), ("1204", 4, false, true));
    }

    #[test]
    fn backspace_and_delete_are_noops_at_the_ends() {
        let mut f = num();
        f.handle(FormEvent::Home); // deselect, cursor -> 0
        f.handle(FormEvent::Backspace); // no-op: nothing before cursor 0
        assert_eq!(buf(&f), ("120", 0, false, false));
        f.handle(FormEvent::End); // cursor -> 3 (end)
        f.handle(FormEvent::Delete); // no-op: nothing after the last digit
        assert_eq!(buf(&f), ("120", 3, false, false));
    }

    #[test]
    fn selecting_from_an_unset_choice_sets_it() {
        use alloc::string::ToString;
        let mut f = FormState::new(
            "T",
            alloc::vec![Field {
                label: "Unset",
                kind: FieldKind::Choice {
                    options: alloc::vec![
                        ChoiceOption {
                            label: "A".to_string(),
                            action: TestOp::On
                        },
                        ChoiceOption {
                            label: "B".to_string(),
                            action: TestOp::Off
                        },
                    ],
                    selected: None,
                },
                restore: Vec::new(),
            }],
        );
        f.handle(FormEvent::Enter); // opens popup, highlight 0
        f.handle(FormEvent::Down);
        let out = f.handle(FormEvent::Enter);
        assert_eq!(out.actions, alloc::vec![TestOp::Off]);
        match &f.fields()[0].kind {
            FieldKind::Choice { selected, .. } => assert_eq!(*selected, Some(1)),
            _ => panic!("not a Choice"),
        }
    }
}
