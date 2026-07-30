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

/// Which button a [`FieldKind::Button`] is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ButtonKind {
    Ok,
    Cancel,
}

/// One selectable value of a [`FieldKind::Choice`].
#[derive(Debug, Clone)]
pub struct ChoiceOption<A> {
    pub label: String,
    pub action: A,
}

/// What a field is and how it behaves.
#[derive(Debug, Clone)]
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
    /// Two-state. Enter flips it and emits the matching action.
    Toggle {
        on: bool,
        on_action: A,
        off_action: A,
    },
    /// A derived value, shown but not editable. Skipped by focus traversal.
    ReadOnly(String),
    Button(ButtonKind),
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
        while btn_start > 0 && matches!(self.fields[btn_start - 1].kind, FieldKind::Button(_)) {
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
        }
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

    /// Index of the first OK button, if the form has one.
    fn ok_button(&self) -> Option<usize> {
        self.fields
            .iter()
            .position(|f| matches!(f.kind, FieldKind::Button(ButtonKind::Ok)))
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
                if self.focused_is_number() {
                    self.on_left();
                } else {
                    self.step_col(-1);
                }
                FormOutcome::nothing()
            }
            FormEvent::Right => {
                if self.focused_is_number() {
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
            FormEvent::Escape => FormOutcome::closing(self.cancel_actions()),
            FormEvent::Enter => self.activate(),
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

    fn focused_is_number(&self) -> bool {
        matches!(
            self.fields.get(self.focus).map(|f| &f.kind),
            Some(FieldKind::Number { .. })
        )
    }

    /// Re-select the focused field if it is a Number (focus-in hook).
    fn reselect_focused(&mut self) {
        if let Some(FieldKind::Number {
            buffer,
            cursor,
            selected,
            ..
        }) = self.focused_kind_mut()
        {
            *selected = true;
            *cursor = buffer.len();
        }
    }

    fn type_char(&mut self, c: char) {
        if !c.is_ascii_digit() {
            return;
        }
        if let Some(FieldKind::Number {
            buffer,
            cursor,
            selected,
            overtype,
            ..
        }) = self.focused_kind_mut()
        {
            if *selected {
                buffer.clear();
                buffer.push(c);
                *selected = false;
                *cursor = buffer.len();
                return;
            }
            let mut tmp = [0u8; 4];
            let s = c.encode_utf8(&mut tmp);
            if *overtype {
                if *cursor < buffer.len() {
                    buffer.replace_range(*cursor..*cursor + 1, s);
                    *cursor += 1;
                } else if buffer.len() < 4 {
                    buffer.push(c);
                    *cursor += 1;
                }
            } else if buffer.len() < 4 {
                buffer.insert(*cursor, c);
                *cursor += 1;
            }
        }
    }

    fn on_left(&mut self) {
        if let Some(FieldKind::Number {
            cursor, selected, ..
        }) = self.focused_kind_mut()
        {
            if *selected {
                *selected = false;
                *cursor = 0;
            } else {
                *cursor = cursor.saturating_sub(1);
            }
        }
    }

    fn on_right(&mut self) {
        if let Some(FieldKind::Number {
            buffer,
            cursor,
            selected,
            ..
        }) = self.focused_kind_mut()
        {
            // Right when selected moves the cursor to the end (CUA spec),
            // rather than relying on the unstated invariant that a selected
            // field's cursor already sits at `buffer.len()` (which the
            // `+ 1` below would otherwise depend on silently).
            if *selected {
                *selected = false;
                *cursor = buffer.len();
            } else {
                *cursor = (*cursor + 1).min(buffer.len());
            }
        }
    }

    fn on_home(&mut self) {
        if let Some(FieldKind::Number {
            cursor, selected, ..
        }) = self.focused_kind_mut()
        {
            *selected = false;
            *cursor = 0;
        }
    }

    fn on_end(&mut self) {
        if let Some(FieldKind::Number {
            buffer,
            cursor,
            selected,
            ..
        }) = self.focused_kind_mut()
        {
            *selected = false;
            *cursor = buffer.len();
        }
    }

    fn on_backspace(&mut self) {
        if let Some(FieldKind::Number {
            buffer,
            cursor,
            selected,
            ..
        }) = self.focused_kind_mut()
        {
            if *selected {
                buffer.clear();
                *selected = false;
                *cursor = 0;
            } else if *cursor > 0 {
                buffer.remove(*cursor - 1);
                *cursor -= 1;
            }
        }
    }

    fn on_delete(&mut self) {
        if let Some(FieldKind::Number {
            buffer,
            cursor,
            selected,
            ..
        }) = self.focused_kind_mut()
        {
            if *selected {
                buffer.clear();
                *selected = false;
                *cursor = 0;
            } else if *cursor < buffer.len() {
                buffer.remove(*cursor);
            }
        }
    }

    fn on_insert(&mut self) {
        if let Some(FieldKind::Number {
            overtype, selected, ..
        }) = self.focused_kind_mut()
        {
            *overtype = !*overtype;
            *selected = false;
        }
    }

    fn activate(&mut self) -> FormOutcome<A> {
        let focus = self.focus;
        // Set by the arms that actually change a value, applied after the
        // borrow of `self.fields` ends.
        let mut changed = false;
        let mut advance_to_ok = false;
        let outcome = match self.fields.get_mut(focus).map(|f| &mut f.kind) {
            Some(FieldKind::Choice { selected, .. }) => {
                self.popup = Some(Popup {
                    field: focus,
                    highlight: selected.unwrap_or(0),
                });
                // Opening the popup changes nothing yet; the Enter that picks
                // an option is what marks the field dirty.
                FormOutcome::nothing()
            }
            Some(FieldKind::Toggle {
                on,
                on_action,
                off_action,
            }) => {
                *on = !*on;
                let a = if *on {
                    on_action.clone()
                } else {
                    off_action.clone()
                };
                changed = true;
                FormOutcome::action(a)
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
                // An empty buffer means nothing was typed: no change, no action,
                // and crucially not dirty either — Enter on an untouched number
                // field must not enrol it in the Cancel restore set.
                let Ok(parsed) = buffer.parse::<u32>() else {
                    // Empty or invalid buffer: nothing to commit.
                    return FormOutcome::nothing();
                };
                let clamped = parsed.clamp(*min, *max);
                // Normalise the live buffer to the clamped value either way.
                *buffer = alloc::format!("{clamped}");
                *cursor = buffer.len();
                *selected = false;
                // Enter always advances to OK so the dialog can be dismissed
                // — including when the value is unchanged. This is what makes
                // the Ctrl+T timing dialog's "open, press Enter to accept the
                // default and dismiss" flow work: a bare Enter must still
                // land on OK so a second Enter closes the form, even though
                // nothing was actually typed. A committed number leaves focus
                // on OK so the flow terminates: the old standalone entry
                // widget closed on Enter, and without this the user is left
                // inside a form whose only obvious exit key (Escape) reverts
                // what they just typed.
                advance_to_ok = true;
                if clamped == *value {
                    // No real change: don't dirty, don't emit — but still
                    // advance to OK (above).
                    FormOutcome::nothing()
                } else {
                    *value = clamped;
                    changed = true;
                    FormOutcome::action(commit(clamped))
                }
            }
            Some(FieldKind::Button(ButtonKind::Ok)) => FormOutcome::closing(Vec::new()),
            Some(FieldKind::Button(ButtonKind::Cancel)) => {
                FormOutcome::closing(self.cancel_actions())
            }
            Some(FieldKind::ReadOnly(_)) | None => FormOutcome::nothing(),
        };
        if changed {
            self.mark_dirty(focus);
        }
        if advance_to_ok {
            if let Some(i) = self.ok_button() {
                self.focus = i;
            }
        }
        outcome
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
            // horizontal and Tab movements have no meaning inside it.
            FormEvent::Char(_)
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
                kind: FieldKind::Button(ButtonKind::Ok),
                restore: Vec::new(),
            },
            Field {
                label: "",
                kind: FieldKind::Button(ButtonKind::Cancel),
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
    fn number_with_an_empty_buffer_emits_nothing() {
        let mut f = form();
        f.set_focus(3);
        let out = f.handle(FormEvent::Enter);
        assert!(out.actions.is_empty());
        assert!(!out.close);
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
                    kind: FieldKind::Button(ButtonKind::Ok),
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
                    kind: FieldKind::Button(ButtonKind::Ok),
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
    fn committing_a_number_moves_focus_to_the_ok_button() {
        let mut f = form();
        f.set_focus(3);
        f.handle(FormEvent::Char('3'));
        f.handle(FormEvent::Char('0'));
        f.handle(FormEvent::Enter);
        assert_eq!(
            f.focus(),
            4,
            "Enter on a number field lands on OK so a second Enter finishes"
        );
        assert!(
            f.handle(FormEvent::Enter).close,
            "so Ctrl+T, type, Enter, Enter completes"
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
                    kind: FieldKind::Button(ButtonKind::Cancel),
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
                    kind: FieldKind::Button(ButtonKind::Ok),
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
    fn enter_on_unchanged_value_advances_focus_to_ok() {
        // Regression for the "open a dialog, press Enter twice to accept the
        // default and dismiss" flow: Enter on an unchanged value must still
        // advance focus to OK, so the second Enter closes the form, even
        // though nothing was actually typed.
        let mut f = num(); // buffer "120" == value 120; fields: [Number, OK]
        let ok_index = f
            .fields()
            .iter()
            .position(|fld| matches!(fld.kind, FieldKind::Button(ButtonKind::Ok)))
            .expect("form has an OK button");
        let out = f.handle(FormEvent::Enter);
        assert!(out.actions.is_empty(), "unchanged value emits no action");
        assert_eq!(
            f.focus(),
            ok_index,
            "Enter on an unchanged value still advances focus to OK"
        );
        // The field was never dirtied, so Escape afterward restores nothing.
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
