//! Writing a result to a file, offered next to the result itself.
//!
//! The view chooses a format and asks; it never opens a file. `oxyn-app` turns
//! the request into a bus command, and the executor streams the shared
//! [`ResultBuffer`](oxyn_data::ResultBuffer) to disk. Nothing is materialised on
//! the way ([I-06](../../../CLAUDE.md#i-06)), and nothing is written on the UI
//! thread ([I-05](../../../CLAUDE.md#i-05)).
//!
//! # The five states ([UX-SPEC](../../../docs/UX-SPEC.md#états-dune-vue))
//!
//! | State | [`ExportPhase`] |
//! |---|---|
//! | Initial | [`Idle`](ExportPhase::Idle), with the [`NotExportable`] reason when there is one |
//! | Running | [`Running`](ExportPhase::Running), with its cancel control |
//! | Populated | [`Written`](ExportPhase::Written) with rows |
//! | Empty | [`Written`](ExportPhase::Written) with zero rows — the result was legitimately empty |
//! | Error | [`Failed`](ExportPhase::Failed) |
//!
//! [`Cancelled`](ExportPhase::Cancelled) sits beside those five: it is neither
//! a failure nor a result, and it exists to name the file left half written —
//! returning to the neutral state would leave a truncated file with no marker.
//!
//! # When the gesture is not offered, and why
//!
//! Two different situations, and merging them is the bug this module was
//! rewritten to remove ([`NotExportable`]):
//!
//! * **the buffer is still filling** —
//!   [`ExportOptions::allow_incomplete`](oxyn_data::ExportOptions) is false by
//!   default, so exporting would fail after the user picked a file;
//! * **the buffer is closed but truncated** — stopped by the row limit or the
//!   memory budget ([I-06](../../../CLAUDE.md#i-06)). It *has* finished
//!   loading, so telling the user to wait would be a sentence that never comes
//!   true, and exporting it would write a file that looks like the whole table.
//!
//! A format the product cannot write yet is shown disabled rather than hidden:
//! [`oxyn_data::is_supported`] decides. Offering it would fail after the file
//! dialog and leave an empty file on disk; hiding it would read as a product
//! that never had it.

use gpui::prelude::*;
use gpui::{
    AnyElement, ClickEvent, Context, EventEmitter, FocusHandle, Focusable, KeyDownEvent,
    SharedString, Window, div,
};
use oxyn_core::ExportFormat;

use crate::theme::Theme;

/// Every format `ExportFormat` names, in the order the screen lists them.
///
/// Written out rather than derived: `ExportFormat` is `#[non_exhaustive]`, so a
/// format added upstream must be placed here deliberately — appearing on screen
/// in whatever order an iterator produced is not a design.
///
/// Not all of them are offered: [`oxyn_data::is_supported`] decides, and the
/// three that are not yet written are shown as unavailable rather than hidden.
/// A format that vanishes from the menu reads as a product that never had it.
pub const KNOWN_FORMATS: [ExportFormat; 8] = [
    ExportFormat::Csv,
    ExportFormat::Tsv,
    ExportFormat::Json,
    ExportFormat::JsonLines,
    ExportFormat::Parquet,
    ExportFormat::ArrowIpc,
    ExportFormat::Sql,
    ExportFormat::Markdown,
];

/// The name a format goes by on screen.
///
/// `ExportFormat`'s own `Display` renders the file extension, which is what a
/// filename needs and not what a menu needs: `md` does not read as "Markdown"
/// and `jsonl` does not read as "JSON lines".
#[must_use]
pub fn format_label(format: ExportFormat) -> &'static str {
    match format {
        ExportFormat::Csv => "CSV",
        ExportFormat::Tsv => "TSV",
        ExportFormat::Json => "JSON",
        ExportFormat::JsonLines => "JSON lines",
        ExportFormat::Parquet => "Parquet",
        ExportFormat::ArrowIpc => "Arrow IPC",
        ExportFormat::Sql => "SQL inserts",
        ExportFormat::Markdown => "Markdown",
        // `ExportFormat` is `#[non_exhaustive]`: a format this version does not
        // know keeps its extension rather than vanishing from the menu.
        _ => "Other",
    }
}

/// What the export bar asks for.
///
/// It emits; it does not export. `oxyn-app` picks the destination and submits
/// the request to the bus ([I-01](../../../CLAUDE.md#i-01)).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum ExportEvent {
    /// The user chose a format and wants to pick a destination.
    Requested(ExportFormat),
    /// The user wants to stop the export under way.
    CancelRequested,
}

/// Why a result cannot be exported.
///
/// Distinct from "no result yet": a truncated result *has* finished loading,
/// and telling the user to wait for it would be a sentence that never comes
/// true.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum NotExportable {
    /// No execution has produced a whole result yet.
    NoResult,
    /// The result stopped short of the rows the query would have returned —
    /// row limit or memory budget. Exporting it would write a file that looks
    /// like the whole table and is not.
    Truncated,
}

impl NotExportable {
    /// What the bar says instead of offering the gesture.
    #[must_use]
    pub const fn message(self) -> &'static str {
        match self {
            Self::NoResult => "Export becomes available once a result has finished loading.",
            Self::Truncated => {
                "This result is truncated, so it cannot be exported: the file would look \
                 like the whole table. Narrow the query, or raise the row limit, and run it again."
            }
        }
    }
}

/// Where an export stands.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
#[non_exhaustive]
pub enum ExportPhase {
    /// Nothing has been exported yet.
    #[default]
    Idle,
    /// A destination has been chosen and the file is being written.
    Running,
    /// The export was stopped. The bytes already written stay on disk: deleting
    /// them here would destroy something the user may want to look at, and
    /// saying nothing would leave a truncated file with no marker.
    Cancelled {
        /// The file left partially written.
        destination: SharedString,
    },
    /// The file was written. `rows == 0` is the empty state, not a failure.
    Written {
        /// Rows written, as the executor counted them.
        rows: usize,
        /// Bytes written.
        bytes: u64,
        /// The destination, as the user named it. A path the user typed is not
        /// a secret; a connection identifier would be
        /// ([I-03](../../../CLAUDE.md#i-03)).
        destination: SharedString,
    },
    /// The export failed. Carries the message as it came, not a paraphrase.
    Failed {
        /// The message, code included.
        message: SharedString,
        /// Can the same export be retried as is? Comes from
        /// [`ErrorClass`](oxyn_core::ErrorClass), never from reading the
        /// message ([I-13](../../../CLAUDE.md#i-13)).
        retryable: bool,
    },
}

/// The export bar shown under a result.
#[derive(Debug)]
pub struct ResultExport {
    focus: FocusHandle,
    phase: ExportPhase,
    /// `None` once a whole result is available; otherwise why it is not, so the
    /// bar says something true instead of failing after the file dialog.
    blocked: Option<NotExportable>,
    /// True while the format list is unfolded.
    picking: bool,
    preview_rows: Option<usize>,
    /// How many grid columns the user has hidden, and the export writes anyway.
    ///
    /// The export carries a result id, not a projection: hiding a column is a
    /// reading convenience and never reaches the file. Saying so is not a
    /// preference — someone who hides `email` before sending a CSV needs to
    /// learn it here rather than from the recipient.
    ///
    /// Whether the export *should* follow the grid instead is an open product
    /// question (`docs/IMPLEMENTATION-PLAN.md`, « les colonnes masquées »). This
    /// caveat is true under either answer, so it settles nothing.
    hidden_columns: usize,
}

impl EventEmitter<ExportEvent> for ResultExport {}

impl Focusable for ResultExport {
    fn focus_handle(&self, _cx: &gpui::App) -> FocusHandle {
        self.focus.clone()
    }
}

impl ResultExport {
    /// An export bar with no result behind it yet.
    pub fn new(cx: &mut Context<'_, Self>) -> Self {
        Self {
            focus: cx.focus_handle(),
            phase: ExportPhase::Idle,
            blocked: Some(NotExportable::NoResult),
            picking: false,
            preview_rows: None,
            hidden_columns: 0,
        }
    }

    /// Where the export stands.
    #[must_use]
    pub fn phase(&self) -> &ExportPhase {
        &self.phase
    }

    /// Labels an export as a bounded table preview, never as the entire table.
    pub fn set_preview_rows(&mut self, rows: usize, cx: &mut Context<'_, Self>) {
        self.preview_rows = Some(rows);
        cx.notify();
    }

    /// Declares how many grid columns are hidden from view.
    ///
    /// Called whenever the visibility of a column changes, not only when a
    /// result arrives: the user hides `email` *then* exports, and a count taken
    /// at the result's arrival would still read zero.
    pub fn set_hidden_columns(&mut self, hidden: usize, cx: &mut Context<'_, Self>) {
        if self.hidden_columns != hidden {
            self.hidden_columns = hidden;
            cx.notify();
        }
    }

    /// How many hidden columns the export will write anyway.
    #[must_use]
    pub const fn hidden_columns(&self) -> usize {
        self.hidden_columns
    }

    /// Is a whole result available to export?
    #[must_use]
    pub const fn is_result_ready(&self) -> bool {
        self.blocked.is_none()
    }

    /// Why the result cannot be exported, if it cannot.
    #[must_use]
    pub const fn blocked(&self) -> Option<NotExportable> {
        self.blocked
    }

    /// Declares whether a whole result is available, and why not when it is not.
    ///
    /// Losing the result folds the format list back: leaving it open would
    /// offer formats for a result that no longer exists.
    pub fn set_result_ready(&mut self, blocked: Option<NotExportable>, cx: &mut Context<'_, Self>) {
        if self.blocked != blocked {
            self.blocked = blocked;
            if blocked.is_some() {
                self.picking = false;
                if self.phase != ExportPhase::Running {
                    self.phase = ExportPhase::Idle;
                }
            }
            cx.notify();
        }
    }

    /// Signals that the destination is chosen and the file is being written.
    pub fn running(&mut self, cx: &mut Context<'_, Self>) {
        self.phase = ExportPhase::Running;
        self.picking = false;
        cx.notify();
    }

    /// Signals a written file.
    pub fn written(
        &mut self,
        rows: usize,
        bytes: u64,
        destination: impl Into<SharedString>,
        cx: &mut Context<'_, Self>,
    ) {
        self.phase = ExportPhase::Written {
            rows,
            bytes,
            destination: destination.into(),
        };
        cx.notify();
    }

    /// Signals a failed export, with the message as the executor gave it.
    pub fn failed(
        &mut self,
        message: impl Into<SharedString>,
        retryable: bool,
        cx: &mut Context<'_, Self>,
    ) {
        self.phase = ExportPhase::Failed {
            message: message.into(),
            retryable,
        };
        cx.notify();
    }

    /// Signals a stopped export, naming the file left partially written.
    pub fn cancelled(&mut self, destination: impl Into<SharedString>, cx: &mut Context<'_, Self>) {
        self.phase = ExportPhase::Cancelled {
            destination: destination.into(),
        };
        cx.notify();
    }

    /// Forgets a finished export. An in-flight export keeps its cancellation control.
    pub fn reset(&mut self, cx: &mut Context<'_, Self>) {
        if self.phase == ExportPhase::Running {
            return;
        }
        self.phase = ExportPhase::Idle;
        self.picking = false;
        // Le compte décrivait le résultat précédent. Le laisser en place ferait
        // annoncer « 2 hidden columns » sur un résultat dont rien n'est masqué :
        // une réserve qui peut être fausse cesse d'être une réserve, et un
        // avertissement pris pour du bruit est ignoré le jour où il est vrai.
        self.hidden_columns = 0;
        cx.notify();
    }

    /// Opens the format choices from a workspace toolbar without submitting an export.
    pub fn show_formats(&mut self, cx: &mut Context<'_, Self>) {
        if self.is_result_ready() && self.phase != ExportPhase::Running {
            self.picking = true;
            cx.notify();
        }
    }

    fn toggle_picker(&mut self, cx: &mut Context<'_, Self>) {
        if self.blocked.is_some() || self.phase == ExportPhase::Running {
            return;
        }
        self.picking = !self.picking;
        cx.notify();
    }

    fn choose(&mut self, format: ExportFormat, cx: &mut Context<'_, Self>) {
        self.picking = false;
        cx.emit(ExportEvent::Requested(format));
        cx.notify();
    }

    /// A control reachable with the keyboard, since GPUI gives none for free
    /// ([ADR-0001](../../../docs/adr/0001-ui-toolkit.md)).
    fn button(
        &self,
        id: &'static str,
        label: impl Into<SharedString>,
        enabled: bool,
        on_activate: impl Fn(&mut Self, &mut Context<'_, Self>) + 'static + Clone,
        cx: &Context<'_, Self>,
    ) -> AnyElement {
        let theme = Theme::of(cx);
        let activate = on_activate.clone();
        div()
            .id(id)
            .when(enabled, |el| el.tab_index(0).cursor_pointer())
            .px_3()
            .py_1()
            .rounded_md()
            .border_1()
            .border_color(theme.colors.border)
            .bg(theme.colors.surface_raised)
            .text_color(if enabled {
                theme.colors.text
            } else {
                theme.colors.text_faint
            })
            .font_family(theme.typography.ui_family.clone())
            .text_size(theme.typography.small_size)
            .when(enabled, |el| {
                el.hover(|style| style.bg(theme.colors.hover))
                    .focus(|style| style.border_color(theme.colors.border_focus))
            })
            .when(enabled, |el| {
                el.on_click(cx.listener(move |this, _: &ClickEvent, _window, cx| {
                    activate(this, cx);
                }))
                .on_key_down(cx.listener(
                    move |this, event: &KeyDownEvent, _window, cx| {
                        if matches!(event.keystroke.key.as_str(), "enter" | "space") {
                            on_activate(this, cx);
                            cx.stop_propagation();
                        }
                    },
                ))
            })
            .child(label.into())
            .into_any_element()
    }

    /// The sentence that stands in for the bar when nothing can be exported.
    ///
    /// The empty state of this surface, and the one a new user meets first: a
    /// bar that showed nothing at all would read as a missing feature.
    ///
    /// Truncation is not drawn in the same register as "no result yet": one is
    /// a neutral wait, the other is the sentence that stops a user from
    /// mistaking a partial file for the whole table, and it only works if it is
    /// read.
    fn hint(&self, blocked: NotExportable, cx: &Context<'_, Self>) -> AnyElement {
        let theme = Theme::of(cx);
        div()
            .text_color(match blocked {
                NotExportable::NoResult => theme.colors.text_faint,
                NotExportable::Truncated => theme.colors.warning,
            })
            .font_family(theme.typography.ui_family.clone())
            .text_size(theme.typography.small_size)
            .child(blocked.message())
            .into_any_element()
    }
}

impl Render for ResultExport {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<'_, Self>) -> impl IntoElement {
        let theme = Theme::of(cx).clone();
        let mut bar = div()
            .id("oxyn-result-export")
            .track_focus(&self.focus)
            .flex()
            .flex_row()
            .flex_wrap()
            .items_center()
            .gap_2()
            .px_3()
            .py_1()
            .flex_none()
            .bg(theme.colors.surface_raised)
            .border_t_1()
            .border_color(theme.colors.grid_line)
            .font_family(theme.typography.ui_family.clone())
            .text_size(theme.typography.small_size)
            .text_color(theme.colors.text_muted);

        match &self.phase {
            ExportPhase::Running => {
                return bar
                    .child("Writing the file…")
                    // The running state always carries its way out (UX-SPEC).
                    .child(self.button(
                        "oxyn-export-cancel",
                        "Cancel",
                        true,
                        |_this, cx| cx.emit(ExportEvent::CancelRequested),
                        cx,
                    ))
                    .into_any_element();
            }
            ExportPhase::Written {
                rows,
                bytes,
                destination,
            } => {
                let summary = if *rows == 0 {
                    format!("No rows written — the result was empty · {destination}")
                } else {
                    format!("{rows} rows written · {bytes} bytes · {destination}")
                };
                bar = bar.child(
                    div()
                        .text_color(theme.colors.success)
                        .child(SharedString::from(summary)),
                );
            }
            ExportPhase::Failed { message, retryable } => {
                bar = bar
                    .child(div().text_color(theme.colors.danger).child(message.clone()))
                    .child(if *retryable {
                        "The export can be retried as is."
                    } else {
                        "Retrying the same export gives the same result."
                    });
            }
            ExportPhase::Cancelled { destination } => {
                bar = bar.child(
                    div()
                        .text_color(theme.colors.warning)
                        .child(SharedString::from(format!(
                            "Export stopped · {destination} was left partially written",
                        ))),
                );
            }
            ExportPhase::Idle => {}
        }

        if let Some(blocked) = self.blocked {
            return bar.child(self.hint(blocked, cx)).into_any_element();
        }

        if let Some(rows) = self.preview_rows {
            bar = bar.child(format!(
                "{rows} preview rows included · Not the entire table"
            ));
        }
        if self.hidden_columns > 0 {
            bar = bar.child(
                div()
                    .text_color(theme.colors.warning)
                    .child(SharedString::from(format!(
                        "{} hidden column{} will still be written",
                        self.hidden_columns,
                        if self.hidden_columns == 1 { "" } else { "s" }
                    ))),
            );
        }
        if self.preview_rows.is_none() {
            bar = bar.child(self.button(
                "oxyn-export-open",
                if self.picking {
                    "Export ▴"
                } else {
                    "Export ▾"
                },
                true,
                |this, cx| this.toggle_picker(cx),
                cx,
            ));
        }

        if self.picking {
            for format in KNOWN_FORMATS {
                // A format Oxyn cannot write yet is shown, disabled, with its
                // extension: hiding it would read as a product that never had
                // it, and offering it would fail after the file dialog, leaving
                // an empty file behind.
                let writable = oxyn_data::is_supported(format);
                bar = bar.child(self.button(
                    format_button_id(format),
                    if writable {
                        format!("{} (.{})", format_label(format), format.extension())
                    } else {
                        format!(
                            "{} (.{}) — not written yet",
                            format_label(format),
                            format.extension()
                        )
                    },
                    writable,
                    move |this, cx| this.choose(format, cx),
                    cx,
                ));
            }
        }

        bar.into_any_element()
    }
}

/// A stable element id per format.
///
/// GPUI needs a `&'static str`; building one from the format at render time
/// would allocate on every frame and change identity with it, which loses focus
/// on the button the user is tabbing through.
const fn format_button_id(format: ExportFormat) -> &'static str {
    match format {
        ExportFormat::Csv => "oxyn-export-csv",
        ExportFormat::Tsv => "oxyn-export-tsv",
        ExportFormat::Json => "oxyn-export-json",
        ExportFormat::JsonLines => "oxyn-export-jsonl",
        ExportFormat::Parquet => "oxyn-export-parquet",
        ExportFormat::ArrowIpc => "oxyn-export-arrow",
        ExportFormat::Sql => "oxyn-export-sql",
        ExportFormat::Markdown => "oxyn-export-markdown",
        _ => "oxyn-export-other",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn chaque_format_offert_a_un_libelle_et_un_identifiant_distincts() {
        // Deux formats partageant un identifiant d'élément se voleraient le
        // focus : le symptôme est une tabulation qui saute un bouton, ce que
        // personne ne relie à un identifiant recopié.
        let mut libelles: Vec<&str> = KNOWN_FORMATS.iter().copied().map(format_label).collect();
        let mut ids: Vec<&str> = KNOWN_FORMATS
            .iter()
            .copied()
            .map(format_button_id)
            .collect();
        libelles.sort_unstable();
        ids.sort_unstable();
        let avant = libelles.len();
        libelles.dedup();
        ids.dedup();
        assert_eq!(
            libelles.len(),
            avant,
            "deux formats portent le même libellé"
        );
        assert_eq!(ids.len(), avant, "deux formats portent le même identifiant");
        assert_eq!(avant, 8, "les huit formats d'ExportFormat sont listés");
    }

    #[test]
    fn aucun_libelle_ne_se_reduit_a_lextension() {
        // `ExportFormat: Display` rend l'extension, qui convient à un nom de
        // fichier et pas à un menu : « md » ne se lit pas « Markdown ».
        for format in KNOWN_FORMATS {
            assert_ne!(
                format_label(format),
                format.extension(),
                "{format} n'est pas nommé, seulement suffixé"
            );
        }
    }

    #[test]
    fn seuls_les_formats_reellement_ecrits_sont_activables() {
        // Le geste s'arrête avant le sélecteur de fichier, pas après : sinon
        // l'utilisateur nomme sa destination, l'écriture échoue, et un fichier
        // vide reste sur son disque.
        let ecrivables = KNOWN_FORMATS
            .iter()
            .filter(|format| oxyn_data::is_supported(**format))
            .count();
        assert!(
            (1..KNOWN_FORMATS.len()).contains(&ecrivables),
            "cette version écrit {ecrivables} des {} formats connus : si elle les \
             écrit tous, ce test et la mention « not written yet » n'ont plus de raison d'être",
            KNOWN_FORMATS.len()
        );
    }

    #[test]
    fn un_resultat_tronque_ne_se_confond_pas_avec_labsence_de_resultat() {
        // Le tronqué a fini de charger : lui dire d'attendre serait une phrase
        // qui ne se réalise jamais, et l'exporter écrirait un fichier qui a
        // l'air d'être toute la table.
        assert_ne!(
            NotExportable::Truncated.message(),
            NotExportable::NoResult.message()
        );
        assert!(NotExportable::Truncated.message().contains("truncated"));
    }
}
