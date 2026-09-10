//! Local editor for positional query parameters.

use std::fmt;

use gpui::prelude::*;
use gpui::{
    App, Context, Entity, EventEmitter, FocusHandle, Focusable, SharedString, Window, div, px,
};
/// Scalar types accepted by the parameter editor.
///
/// Defined in `oxyn_core::value` — the same enum a driver's type table
/// matches on — and re-exported here so `oxyn_ui::ParameterType` keeps
/// existing for callers that already name it. `oxyn-ui` gains no new
/// dependency from this: the parsing that needs `chrono`/`uuid`/`serde_json`
/// stays in `oxyn-core`, which already depends on them.
pub use oxyn_core::ParameterType;
use oxyn_core::ScalarValue;

use crate::controls::{ControlState, ControlTone, control};
use crate::select_field::SelectField;
use crate::text_field::TextField;
use crate::theme::Theme;

fn options() -> Vec<SharedString> {
    ParameterType::ALL
        .iter()
        .map(|kind| kind.label().into())
        .collect()
}

/// Why a parameter value could not be converted. It deliberately carries no input text.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ParameterErrorKind {
    InvalidValue,
    TooManyParameters,
    TooManyBytes,
}

/// A value conversion error identified only by position and expected type.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ParameterError {
    pub position: usize,
    pub parameter_type: ParameterType,
    pub kind: ParameterErrorKind,
}

impl fmt::Display for ParameterError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self.kind {
            ParameterErrorKind::TooManyParameters => write!(
                f,
                "parameter {} ({}) exceeds the maximum count of 128",
                self.position,
                self.parameter_type.label()
            ),
            ParameterErrorKind::TooManyBytes => write!(
                f,
                "parameter {} ({}) exceeds the memory limit",
                self.position,
                self.parameter_type.label()
            ),
            ParameterErrorKind::InvalidValue => write!(
                f,
                "parameter {} is not a valid {} value",
                self.position,
                self.parameter_type.label()
            ),
        }
    }
}

const MAX_PARAMETERS: usize = 128;
const MAX_VALUE_BYTES: usize = 1_048_576;

/// Parses `text` as a value of `kind`, discarding the parsing detail: this
/// crate only ever needs "did it work", and `oxyn_core::ParameterParseError`
/// is not `Display`-worthy on its own here (the position and type carried by
/// [`ParameterError`] already say everything the caller needs).
fn parse_value(kind: ParameterType, text: &str) -> Result<ScalarValue, ParameterErrorKind> {
    kind.parse(text)
        .map_err(|_| ParameterErrorKind::InvalidValue)
}

struct ParameterRow {
    /// Identity that survives the removal of a neighbour.
    ///
    /// A row's subscriptions outlive every reordering, so they cannot hold a
    /// position: removing `$1` would leave the callbacks of `$2` and `$3`
    /// pointing one row too far, and a type picked on one line would land on
    /// the next — invisibly, because each `SelectField` shows its own state.
    id: u64,
    /// The declared type, and the **only** thing that says whether this
    /// parameter is `NULL`. A second flag meaning the same thing is a second
    /// thing to forget to update.
    kind: ParameterType,
    value: Entity<TextField>,
    selector: Entity<SelectField>,
    invalid: bool,
}

/// In-memory positional parameter editor. Positions are one-based (`$1`, `$2`, ...).
pub struct ParameterEditor {
    focus: FocusHandle,
    rows: Vec<ParameterRow>,
    next_id: u64,
}

impl fmt::Debug for ParameterEditor {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ParameterEditor")
            .field("len", &self.rows.len())
            .finish_non_exhaustive()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ParameterEditorEvent {
    Changed,
}
impl EventEmitter<ParameterEditorEvent> for ParameterEditor {}
impl Focusable for ParameterEditor {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus.clone()
    }
}

impl ParameterEditor {
    pub fn new(cx: &mut Context<'_, Self>) -> Self {
        Self {
            focus: cx.focus_handle(),
            rows: Vec::new(),
            next_id: 0,
        }
    }

    /// Where the row with this identity currently sits, if it is still there.
    fn position_of(&self, id: u64) -> Option<usize> {
        self.rows.iter().position(|row| row.id == id)
    }

    /// Adds one positional parameter, up to the product limit.
    pub fn add_parameter(&mut self, cx: &mut Context<'_, Self>) {
        if self.rows.len() >= MAX_PARAMETERS {
            return;
        }
        let id = self.next_id;
        self.next_id = self.next_id.wrapping_add(1);
        let value = cx.new(|cx| TextField::new(String::new(), false, cx).with_managed_tab_order());
        value.update(cx, |field, cx| {
            field.set_clipboard_export_allowed(false);
            field.set_read_only(true, cx);
        });
        let selector = cx.new(|cx| SelectField::new(options(), 0, cx));
        cx.subscribe(&value, move |editor, field, event, cx| {
            if matches!(event, crate::FieldEvent::Changed) {
                if let Some(row) = editor
                    .position_of(id)
                    .and_then(|at| editor.rows.get_mut(at))
                {
                    row.invalid = row.kind != ParameterType::Null
                        && parse_value(row.kind, field.read(cx).text()).is_err();
                }
                editor.emit_changed(cx);
            }
        })
        .detach();
        cx.subscribe(&selector, move |editor, _field, event, cx| {
            let crate::SelectEvent { index: type_index } = *event;
            let Some(kind) = ParameterType::ALL.get(type_index).copied() else {
                return;
            };
            let Some(at) = editor.position_of(id) else {
                return;
            };
            editor.apply_parameter_type(at, kind, cx);
            editor.emit_changed(cx);
        })
        .detach();
        self.rows.push(ParameterRow {
            id,
            kind: ParameterType::Null,
            value,
            selector,
            invalid: false,
        });
        self.emit_changed(cx);
    }

    #[must_use]
    pub const fn len(&self) -> usize {
        self.rows.len()
    }

    /// Whether there is no row at all — meaning no bound value will leave
    /// with the request, positional or otherwise.
    #[must_use]
    pub const fn is_empty(&self) -> bool {
        self.rows.is_empty()
    }

    /// Returns the position-th row's value field, if that row exists.
    ///
    /// For orchestration that needs a stable handle on it — giving the first
    /// field focus when the panel opens, or, in a test, writing a value
    /// directly and checking what actually reaches the driver bound.
    #[must_use]
    pub fn value_field(&self, index: usize) -> Option<Entity<TextField>> {
        self.rows.get(index).map(|row| row.value.clone())
    }

    /// Changes the type of the position-th row, exactly as picking it from
    /// the row's own `SelectField` would — the picker is brought in line too,
    /// through the one function both paths share. Does nothing — including
    /// emitting no event — if `index` is out of range.
    pub fn set_parameter_type(
        &mut self,
        index: usize,
        kind: ParameterType,
        cx: &mut Context<'_, Self>,
    ) {
        if !self.apply_parameter_type(index, kind, cx) {
            return;
        }
        self.emit_changed(cx);
    }

    /// Single place that keeps `kind`, `invalid`, the picker's own selection
    /// and the value field's read-only flag in sync for one row, whichever of
    /// the `SelectField` subscription or [`Self::set_parameter_type`] triggered
    /// the change. Returns whether `index` was in range; callers decide from
    /// that whether a `Changed` event follows.
    fn apply_parameter_type(
        &mut self,
        index: usize,
        kind: ParameterType,
        cx: &mut Context<'_, Self>,
    ) -> bool {
        let Some(row) = self.rows.get_mut(index) else {
            return false;
        };
        row.kind = kind;
        row.invalid = false;
        // A `NULL` parameter has no text to type: leaving the field editable
        // would let someone fill in a value the request will not carry.
        let null = kind == ParameterType::Null;
        row.value
            .update(cx, |field, cx| field.set_read_only(null, cx));
        let selected = ParameterType::ALL.iter().position(|other| *other == kind);
        if let Some(selected) = selected {
            row.selector
                .update(cx, |picker, cx| picker.set_selected(selected, cx));
        }
        true
    }

    /// Converts all fields without exposing their text in an error.
    pub fn values(&self, cx: &App) -> Result<Vec<ScalarValue>, ParameterError> {
        if self.rows.len() > MAX_PARAMETERS {
            return Err(ParameterError {
                position: self.rows.len(),
                parameter_type: ParameterType::Text,
                kind: ParameterErrorKind::TooManyParameters,
            });
        }
        let mut total: usize = 0;
        let mut values = Vec::with_capacity(self.rows.len());
        for (index, row) in self.rows.iter().enumerate() {
            if row.kind == ParameterType::Null {
                values.push(ScalarValue::Null);
                continue;
            }
            let text = row.value.read(cx).text();
            total = total.saturating_add(text.len());
            if total > MAX_VALUE_BYTES {
                return Err(ParameterError {
                    position: index + 1,
                    parameter_type: row.kind,
                    kind: ParameterErrorKind::TooManyBytes,
                });
            }
            values.push(parse_value(row.kind, text).map_err(|kind| ParameterError {
                position: index + 1,
                parameter_type: row.kind,
                kind,
            })?);
        }
        Ok(values)
    }

    fn emit_changed(&mut self, cx: &mut Context<'_, Self>) {
        cx.emit(ParameterEditorEvent::Changed);
        cx.notify();
    }
}

impl Render for ParameterEditor {
    fn render(&mut self, _: &mut Window, cx: &mut Context<'_, Self>) -> impl IntoElement {
        let theme = Theme::of(cx).clone();
        let mut body = div()
            .id("query-parameters")
            .flex()
            .flex_col()
            .gap(theme.spacing.small);
        for (index, row) in self.rows.iter().enumerate() {
            let position = index + 1;
            let invalid = row.invalid;
            // The listeners address the row by identity, not by position: a
            // removal renumbers everything below it, and a listener that kept
            // an index would act on the row that took the place of its own.
            let id = row.id;
            let remove = control(
                ("parameter-remove", position),
                ControlState::Enabled,
                ControlTone::Neutral,
                &theme,
                cx.listener(move |this, _, _, cx| {
                    let Some(at) = this.position_of(id) else {
                        return;
                    };
                    this.rows.remove(at);
                    this.emit_changed(cx);
                }),
            )
            .child("Remove");
            // Clears the text only. The type — `NULL` included — belongs to the
            // picker, and a second control that also set it would be a second
            // thing to keep in agreement with it.
            let clear = control(
                ("parameter-clear", position),
                ControlState::Enabled,
                ControlTone::Neutral,
                &theme,
                cx.listener(move |this, _, _, cx| {
                    let Some(row) = this.position_of(id).and_then(|at| this.rows.get_mut(at))
                    else {
                        return;
                    };
                    row.invalid = false;
                    row.value
                        .update(cx, |field, cx| field.set_text(String::new(), cx));
                    this.emit_changed(cx);
                }),
            )
            .child("Clear");
            body = body.child(
                div()
                    .id(position)
                    .flex()
                    .items_center()
                    .gap(theme.spacing.tiny)
                    .child(format!("{position}"))
                    .child(row.selector.clone())
                    .child(div().w(px(260.)).child(row.value.clone()))
                    .child(clear)
                    .child(remove)
                    .when(invalid, |el| el.child("Invalid value")),
            );
        }
        body.child(
            control(
                "parameter-add",
                if self.rows.len() < MAX_PARAMETERS {
                    ControlState::Enabled
                } else {
                    ControlState::Disabled
                },
                ControlTone::Primary,
                &theme,
                cx.listener(|this, _, _, cx| {
                    this.add_parameter(cx);
                }),
            )
            .child("Add"),
        )
        .into_any_element()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn parser_rejects_invalid_hex_without_copying_input() {
        let error = parse_value(ParameterType::Bytes, "not-hex").expect_err("invalid hex");
        assert_eq!(error, ParameterErrorKind::InvalidValue);
    }
    #[test]
    fn parser_keeps_decimal_as_text() {
        assert_eq!(
            parse_value(ParameterType::Decimal, "0.10").expect("decimal"),
            ScalarValue::Decimal("0.10".into())
        );
    }

    /// These six types used to be refused outright with `UnsupportedType` —
    /// the very variant this fix removes, because the selector was already
    /// offering them. This is the regression test for that fix.
    #[test]
    fn the_previously_unsupported_types_now_parse() {
        assert!(parse_value(ParameterType::Uuid, "550e8400-e29b-41d4-a716-446655440000").is_ok());
        assert!(parse_value(ParameterType::Date, "2024-01-15").is_ok());
        assert!(parse_value(ParameterType::Time, "13:45:00").is_ok());
        assert!(parse_value(ParameterType::Timestamp, "2024-01-15T13:45:00Z").is_ok());
        assert!(parse_value(ParameterType::TimestampNaive, "2024-01-15T13:45:00").is_ok());
        assert!(parse_value(ParameterType::Json, "{\"a\":1}").is_ok());
    }

    #[test]
    fn the_type_picker_and_the_editor_agree_on_the_type_list() {
        // `options()` and the selector's index lookup both derive from
        // `ParameterType::ALL` now, so they cannot drift apart again.
        assert_eq!(options().len(), ParameterType::ALL.len());
    }

    #[test]
    fn the_product_limits_are_unchanged() {
        assert_eq!(MAX_PARAMETERS, 128);
        assert_eq!(MAX_VALUE_BYTES, 1_048_576);
    }

    #[gpui::test]
    fn is_empty_reflects_whether_any_row_exists(cx: &mut gpui::TestAppContext) {
        let (view, cx) = cx.add_window_view(|_, cx| ParameterEditor::new(cx));
        assert!(view.read_with(cx, |editor, _| editor.is_empty()));
        view.update(cx, |editor, cx| editor.add_parameter(cx));
        assert!(!view.read_with(cx, |editor, _| editor.is_empty()));
    }

    #[gpui::test]
    fn value_field_is_available_only_within_range(cx: &mut gpui::TestAppContext) {
        let (view, cx) = cx.add_window_view(|_, cx| ParameterEditor::new(cx));
        assert!(
            view.read_with(cx, |editor, _| editor.value_field(0))
                .is_none(),
            "no row yet"
        );
        view.update(cx, |editor, cx| editor.add_parameter(cx));
        assert!(
            view.read_with(cx, |editor, _| editor.value_field(0))
                .is_some()
        );
        assert!(
            view.read_with(cx, |editor, _| editor.value_field(1))
                .is_none(),
            "only one row was added"
        );
    }

    /// The path `oxyn-app` uses to test that a typed value reaches the
    /// driver bound: write through the field handle, change the type through
    /// `set_parameter_type`, then read back what `values()` would send.
    #[gpui::test]
    fn set_parameter_type_changes_what_values_returns(cx: &mut gpui::TestAppContext) {
        let (view, cx) = cx.add_window_view(|_, cx| ParameterEditor::new(cx));
        view.update(cx, |editor, cx| editor.add_parameter(cx));
        assert_eq!(
            view.read_with(cx, |editor, cx| editor.values(cx)),
            Ok(vec![ScalarValue::Null]),
            "a fresh row starts out NULL"
        );

        let field = view
            .read_with(cx, |editor, _| editor.value_field(0))
            .expect("row exists");
        view.update(cx, |editor, cx| {
            editor.set_parameter_type(0, ParameterType::Int64, cx);
        });
        field.update(cx, |field, cx| field.set_text("42".into(), cx));

        assert_eq!(
            view.read_with(cx, |editor, cx| editor.values(cx)),
            Ok(vec![ScalarValue::Int64(42)])
        );
    }

    #[gpui::test]
    fn set_parameter_type_does_nothing_out_of_range(cx: &mut gpui::TestAppContext) {
        let changed = std::rc::Rc::new(std::cell::Cell::new(0_u32));
        let observed = changed.clone();
        let (view, cx) = cx.add_window_view(|_, cx| {
            let editor = ParameterEditor::new(cx);
            cx.subscribe(&cx.entity(), move |_, _, _: &ParameterEditorEvent, _| {
                observed.set(observed.get() + 1)
            })
            .detach();
            editor
        });
        view.update(cx, |editor, cx| editor.add_parameter(cx));
        let before = changed.get();

        view.update(cx, |editor, cx| {
            editor.set_parameter_type(1, ParameterType::Int64, cx);
        });

        assert_eq!(changed.get(), before, "out of range: no event fires");
        assert_eq!(
            view.read_with(cx, |editor, cx| editor.values(cx)),
            Ok(vec![ScalarValue::Null]),
            "out of range: the existing row is untouched"
        );
    }

    /// Removing a row must not make its neighbours' controls act on each other.
    ///
    /// The subscriptions of a row outlive every renumbering. When they held a
    /// position, deleting `$1` left the picker of the new `$1` writing into
    /// `$2` — and nothing showed it, because each picker displays its own
    /// state. The value bound then differed from the type on screen.
    #[gpui::test]
    fn removing_a_row_does_not_shift_what_its_neighbours_control(cx: &mut gpui::TestAppContext) {
        let (view, cx) = cx.add_window_view(|_, cx| ParameterEditor::new(cx));
        view.update(cx, |editor, cx| {
            for _ in 0..3 {
                editor.add_parameter(cx);
            }
            for index in 0..3 {
                editor.set_parameter_type(index, ParameterType::Text, cx);
                if let Some(field) = editor.value_field(index) {
                    field.update(cx, |field, cx| field.set_text(format!("row-{index}"), cx));
                }
            }
            editor.rows.remove(0);
        });
        cx.run_until_parked();

        // Ce qui reste, c'est « row-1 » puis « row-2 ». Changer le type de la
        // première ligne restante ne doit toucher qu'elle.
        view.update(cx, |editor, cx| {
            editor.set_parameter_type(0, ParameterType::Int64, cx);
            if let Some(field) = editor.value_field(0) {
                field.update(cx, |field, cx| field.set_text("7".into(), cx));
            }
        });
        cx.run_until_parked();
        view.read_with(cx, |editor, cx| {
            let values = editor.values(cx).expect("les deux lignes se convertissent");
            assert_eq!(
                values,
                vec![ScalarValue::Int64(7), ScalarValue::Text("row-2".into())],
                "la voisine garde son type et sa valeur"
            );
        });
    }

    /// A typed value must never be sent as `NULL` without saying so.
    ///
    /// A fresh row is `NULL`, and its field is read-only for exactly that
    /// reason: there is no way to type a value that the request would then
    /// drop. Choosing a type is what opens the field.
    #[gpui::test]
    fn a_row_stays_null_until_a_type_is_chosen(cx: &mut gpui::TestAppContext) {
        let (view, cx) = cx.add_window_view(|_, cx| ParameterEditor::new(cx));
        view.update(cx, |editor, cx| editor.add_parameter(cx));
        cx.run_until_parked();
        let field = view
            .read_with(cx, |editor, _| editor.value_field(0))
            .expect("champ de valeur");
        assert!(
            field.read_with(cx, |field, _| field.is_read_only()),
            "une ligne NULL n'accepte pas de saisie"
        );
        view.read_with(cx, |editor, cx| {
            assert_eq!(editor.values(cx).expect("valeurs"), vec![ScalarValue::Null]);
        });

        view.update(cx, |editor, cx| {
            editor.set_parameter_type(0, ParameterType::Int64, cx);
        });
        cx.run_until_parked();
        assert!(!field.read_with(cx, |field, _| field.is_read_only()));
        field.update(cx, |field, cx| field.set_text("4242".into(), cx));
        cx.run_until_parked();
        view.read_with(cx, |editor, cx| {
            assert_eq!(
                editor.values(cx).expect("valeurs"),
                vec![ScalarValue::Int64(4242)],
                "la valeur saisie part telle quelle, jamais remplacée par NULL"
            );
        });
    }
}
