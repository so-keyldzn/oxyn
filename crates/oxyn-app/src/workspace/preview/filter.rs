//! What the preview bar of Figma `190:1618` asks of a relation.
//!
//! Two halves that do not look alike ([ADR-0020]). The **predicate** is SQL the
//! user writes: the field carries a draft, and only `Apply` turns that draft
//! into a read. The **sort** is structured: a rank in a menu built from the
//! columns the catalog declares, so Oxyn never composes an identifier it did
//! not read from the catalog itself.
//!
//! A page is a third thing again, and the one that fails silently when it is
//! wrong: an `OFFSET` over an order the server does not guarantee shows a row
//! twice and hides another, with nothing to say so. It is therefore offered
//! only when the whole sequence — the page on screen included — came from one
//! total order.
//!
//! The drawing lives in the `view` child module: it reads private workspace
//! state, which a child module can still see, and keeping both here would make
//! one file carry two subjects.
//!
//! [ADR-0020]: ../../../../docs/adr/0020-apercu-trie-filtre-parcouru.md

use super::*;
use oxyn_catalog::model::Relation;
use oxyn_core::{PreviewShape, PreviewSort};
use oxyn_ui::{FieldEvent, GridState, SelectEvent, SelectField, TextField};

#[cfg(test)]
mod tests;
mod view;

/// Rank 0 of the sort menu: whatever order the server happens to return.
const UNSORTED: &str = "Unsorted";

/// The state behind the filter, sort and page controls of one preview.
///
/// Grouped rather than spread over six workspace fields: `applied` and
/// `requested` only mean anything next to each other, and separating them
/// would invite the next caller to update one and forget the other.
pub(in crate::workspace) struct PreviewControls {
    /// The `WHERE` draft. Typing in it reads nothing: `Apply` does.
    pub(in crate::workspace) predicate: Entity<TextField>,
    /// The sort menu. Ranks are what `SelectField` reports back.
    pub(in crate::workspace) sort: Entity<SelectField>,
    /// Rank → what the command will carry; rank 0 is « no sort ».
    pub(in crate::workspace) sort_choices: Vec<Option<PreviewSort>>,
    /// Whether the sort menu is unfolded under the bar.
    pub(in crate::workspace) sort_open: bool,
    /// Does the catalog declare a unique key for the selected relation?
    ///
    /// `None` means the relation has not been described yet — which is not the
    /// same as having no key, and neither one lets a page be offered.
    pub(in crate::workspace) unique_key: Option<bool>,
    /// The shape the rows currently on screen came from.
    pub(in crate::workspace) applied: PreviewShape,
    /// The shape the read in flight asked for; committed only on its answer.
    pub(in crate::workspace) requested: PreviewShape,
}

impl std::fmt::Debug for PreviewControls {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PreviewControls")
            .field("sort_choices", &self.sort_choices.len())
            .field("sort_open", &self.sort_open)
            .finish_non_exhaustive()
    }
}

impl PreviewControls {
    /// Creates both fields once and subscribes to what the user accepts.
    ///
    /// Called from the workspace constructor alone: the subscriptions belong to
    /// that entity, and a second pair would leave two live paths to the same
    /// read.
    pub(in crate::workspace) fn new(cx: &mut Context<'_, Workspace>) -> Self {
        let predicate = cx.new(|cx| TextField::new(String::new(), false, cx));
        cx.subscribe(
            &predicate,
            |this: &mut Workspace, _, event: &FieldEvent, cx| {
                match event {
                    // Enter in the field is Apply, and nothing else in
                    // `FieldEvent` may read: a keystroke that executes on its
                    // own is the behaviour this bar refuses
                    // ([UX-SPEC](../../../../docs/UX-SPEC.md#filtrer-trier-parcourir)).
                    FieldEvent::Submit => this.apply_preview_predicate(cx),
                    // The field swallows Escape, so the workspace shortcut never
                    // sees it: without this, the way out of the running state
                    // disappears for whoever is standing in the field.
                    FieldEvent::Escape => this.cancel_preview(cx),
                    _ => {}
                }
            },
        )
        .detach();
        let sort = cx.new(|cx| SelectField::new(vec![UNSORTED.into()], 0, cx));
        cx.subscribe(&sort, |this: &mut Workspace, _, event: &SelectEvent, cx| {
            this.choose_preview_sort(event.index, cx);
        })
        .detach();
        Self {
            predicate,
            sort,
            sort_choices: vec![None],
            sort_open: false,
            unique_key: None,
            applied: PreviewShape::unordered(),
            requested: PreviewShape::unordered(),
        }
    }
}

/// The five states of the preview area ([UX-SPEC](../../../../docs/UX-SPEC.md#états-dune-vue)).
#[derive(Debug, Clone, PartialEq, Eq)]
pub(in crate::workspace) enum PreviewState {
    /// Nothing has been read yet, or the last read was abandoned.
    Initial,
    /// A read is on the wire. Cancelling it reaches the server.
    Loading,
    /// Rows are on screen.
    Loaded {
        /// How many rows the buffer holds.
        rows: usize,
    },
    /// The read succeeded and returned nothing.
    Empty {
        /// Whether a predicate was in force — an empty answer and an empty
        /// table are not the same fact, and only one of them is about the
        /// filter the user just wrote.
        filtered: bool,
    },
    /// The server refused, in its own words.
    Failed {
        /// The message as received, code included.
        message: String,
    },
}

/// Which state the preview is in, from the three facts that decide it.
///
/// A read in flight outranks everything: it is the most recent thing the user
/// did, and it is the state that must carry a way out.
///
/// A cancelled grid reads as [`PreviewState::Initial`]: nothing is being shown,
/// and the notice next door is what says the read was stopped rather than never
/// started.
fn preview_state_from(active: bool, grid: &GridState, filtered: bool) -> PreviewState {
    if active {
        return PreviewState::Loading;
    }
    match grid {
        GridState::Idle | GridState::Starting | GridState::Cancelled => PreviewState::Initial,
        GridState::Failed { message, .. } => PreviewState::Failed {
            message: message.to_string(),
        },
        GridState::Streaming(buffer) if buffer.row_count() == 0 && buffer.is_complete() => {
            PreviewState::Empty { filtered }
        }
        GridState::Streaming(buffer) => PreviewState::Loaded {
            rows: buffer.row_count(),
        },
        // `GridState` is `#[non_exhaustive]`: a state added later must not be
        // read here as « rows are on screen », which is the only reading that
        // would let a page control appear over something else entirely.
        _ => PreviewState::Initial,
    }
}

/// What the page controls may offer right now.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(in crate::workspace) enum Pagination {
    /// Nothing was ordered, so no page after the first would mean anything.
    NeedsOrder,
    /// No unique key is known, so no order can be made total.
    NoUniqueKey,
    /// Each direction is offered only when it leads somewhere.
    Ready {
        /// There is a page before this one.
        previous: bool,
        /// This page came back full, so there may be one after it.
        next: bool,
    },
}

/// Whether — and which — page controls the status line may show.
///
/// The order is checked against the shape **the rows on screen came from**, not
/// the one being typed: two consecutive pages only fail to overlap when both
/// were composed from the same total order. That is the whole reason a page is
/// offered after a sort and not before it: the plain first preview has no order
/// at all, so its « next » page would be the second page of an order the user
/// never saw ([ADR-0020](../../../../docs/adr/0020-apercu-trie-filtre-parcouru.md)).
fn pagination_from(applied: &PreviewShape, unique_key: Option<bool>, rows: usize) -> Pagination {
    if !applied.needs_total_order() {
        return Pagination::NeedsOrder;
    }
    // An undescribed relation is not a relation without a key: neither one lets
    // a page be offered, and neither is guessed at.
    if unique_key != Some(true) {
        return Pagination::NoUniqueKey;
    }
    Pagination::Ready {
        previous: applied.offset > 0,
        // A short page is the last one. Offering « next » there would spend a
        // read to show nothing.
        next: rows >= PREVIEW_ROWS as usize,
    }
}

/// The menu entries, from the columns the catalog declares.
///
/// One entry per column and per direction, so that one accepted choice is one
/// read: a separate direction toggle would be a control that does nothing while
/// no column is chosen.
fn sort_choices_from(relation: Option<&Relation>) -> Vec<Option<PreviewSort>> {
    let mut choices = vec![None];
    let Some(relation) = relation else {
        return choices;
    };
    for field in &relation.fields {
        choices.push(Some(PreviewSort::ascending(field.name.clone())));
        choices.push(Some(PreviewSort::descending(field.name.clone())));
    }
    choices
}

/// The menu labels, by rank. Never an identifier of anything but a column.
fn sort_labels(choices: &[Option<PreviewSort>]) -> Vec<gpui::SharedString> {
    choices
        .iter()
        .map(|choice| match choice {
            None => gpui::SharedString::from(UNSORTED),
            Some(sort) => gpui::SharedString::from(format!(
                "{} · {}",
                sort.column,
                if sort.descending {
                    "Descending"
                } else {
                    "Ascending"
                }
            )),
        })
        .collect()
}

/// The rank of the sort the rows on screen came from. Rank 0 when none.
///
/// The shape carries a vector because a driver completes it with the unique
/// key; the bar only ever asks for one column, so only the first is matched.
fn sort_rank(choices: &[Option<PreviewSort>], applied: &PreviewShape) -> usize {
    let Some(sort) = applied.sort.first() else {
        return 0;
    };
    choices
        .iter()
        .position(|choice| choice.as_ref() == Some(sort))
        .unwrap_or(0)
}

/// Whether the field holds something other than what produced the rows.
fn draft_differs(draft: &str, applied: Option<&str>) -> bool {
    draft.trim() != applied.unwrap_or_default()
}

impl Workspace {
    /// Whether this session can filter a preview at all.
    ///
    /// Without the capability there is no field — not a greyed one: a control
    /// that cannot act is a promise the driver does not keep
    /// ([ADR-0003](../../../../docs/adr/0003-driver-capabilities.md)).
    pub(in crate::workspace) fn preview_filter_available(&self) -> bool {
        self.preview_available() && self.capabilities.contains(Capabilities::PREVIEW_FILTER)
    }

    /// Whether this session can order a preview at all.
    pub(in crate::workspace) fn preview_sort_available(&self) -> bool {
        self.preview_available() && self.capabilities.contains(Capabilities::PREVIEW_SORT)
    }

    /// Whether the bar of Figma `190:1618` exists for this session.
    pub(in crate::workspace) fn preview_bar_available(&self) -> bool {
        self.preview_filter_available() || self.preview_sort_available()
    }

    /// Which state the preview area is in right now.
    pub(in crate::workspace) fn preview_state(&self, cx: &gpui::App) -> PreviewState {
        preview_state_from(
            self.preview_active.is_some(),
            self.preview_grid.read(cx).state(),
            self.preview_controls.applied.predicate().is_some(),
        )
    }

    /// What the page controls may offer, given the rows on screen.
    pub(in crate::workspace) fn preview_pagination(&self, cx: &gpui::App) -> Pagination {
        let rows = self
            .preview_grid
            .read(cx)
            .state()
            .buffer()
            .map_or(0, |buffer| buffer.row_count());
        pagination_from(
            &self.preview_controls.applied,
            self.preview_controls.unique_key,
            rows,
        )
    }

    /// Refills the sort menu from the catalog already in memory.
    ///
    /// Reads the cache once per catalog refresh and per object selection, never
    /// per frame: a lock taken while drawing would put the 8 ms budget at its
    /// mercy, and blocking on it would be a stall on the UI thread
    /// ([I-05](../../../../CLAUDE.md#i-05)). A miss on the lock keeps what the
    /// menu had; the next refresh comes back here.
    pub(in crate::workspace) fn refresh_preview_sort_choices(
        &mut self,
        cx: &mut Context<'_, Self>,
    ) {
        let (choices, unique_key) = {
            let Some(cache) = self.catalog_cache.try_read() else {
                return;
            };
            let relation = self
                .selected_path
                .as_ref()
                .and_then(|path| cache.relation(path));
            (
                sort_choices_from(relation),
                relation.map(|relation| !relation.primary_key().is_empty()),
            )
        };
        let changed = choices != self.preview_controls.sort_choices
            || unique_key != self.preview_controls.unique_key;
        self.preview_controls.sort_choices = choices;
        self.preview_controls.unique_key = unique_key;
        let labels = sort_labels(&self.preview_controls.sort_choices);
        let rank = sort_rank(
            &self.preview_controls.sort_choices,
            &self.preview_controls.applied,
        );
        // The entity is kept, so the keyboard focus is kept with it: a catalog
        // that finishes loading while the user is on the menu must not take the
        // control away from under them.
        self.preview_controls
            .sort
            .update(cx, |field, cx| field.set_options(labels, rank, cx));
        if changed {
            cx.notify();
        }
    }

    /// Puts the bar back where a freshly selected object finds it.
    pub(in crate::workspace) fn reset_preview_controls(&mut self, cx: &mut Context<'_, Self>) {
        self.preview_controls.sort_open = false;
        self.preview_controls.applied = PreviewShape::unordered();
        self.preview_controls.requested = PreviewShape::unordered();
        self.preview_controls
            .predicate
            .update(cx, |field, cx| field.set_text(String::new(), cx));
        self.refresh_preview_sort_choices(cx);
    }

    /// Runs a new read with the text in the field. This is `Apply`.
    ///
    /// The page goes back to the first: rows 201 to 400 of a set the predicate
    /// just changed are not the second page of anything the user asked for.
    pub(in crate::workspace) fn apply_preview_predicate(&mut self, cx: &mut Context<'_, Self>) {
        if !self.preview_filter_available() {
            return;
        }
        let text = self.preview_controls.predicate.read(cx).text().to_owned();
        let shape = PreviewShape {
            sort: self.preview_controls.applied.sort.clone(),
            predicate: Some(text),
            offset: 0,
        };
        self.reshape_preview(shape, cx);
    }

    /// Runs a new read ordered by the accepted rank.
    ///
    /// Carries the predicate **in force**, not the one being typed: a sort must
    /// not apply a filter the user never pressed `Apply` for.
    pub(in crate::workspace) fn choose_preview_sort(
        &mut self,
        rank: usize,
        cx: &mut Context<'_, Self>,
    ) {
        if !self.preview_sort_available() {
            return;
        }
        let Some(choice) = self.preview_controls.sort_choices.get(rank).cloned() else {
            return;
        };
        // The field committed the rank the moment it was accepted; the menu must
        // keep showing what produced the rows until the server answers
        // ([UX-SPEC](../../../../docs/UX-SPEC.md#ce-qui-nest-jamais-optimiste)).
        let confirmed = sort_rank(
            &self.preview_controls.sort_choices,
            &self.preview_controls.applied,
        );
        self.preview_controls
            .sort
            .update(cx, |field, cx| field.set_selected(confirmed, cx));
        let shape = PreviewShape {
            sort: choice.into_iter().collect(),
            predicate: self.preview_controls.applied.predicate.clone(),
            offset: 0,
        };
        self.reshape_preview(shape, cx);
    }

    /// Runs the read for the page before or after the one on screen.
    pub(in crate::workspace) fn page_preview(&mut self, forward: bool, cx: &mut Context<'_, Self>) {
        let Pagination::Ready { previous, next } = self.preview_pagination(cx) else {
            return;
        };
        if forward && !next || !forward && !previous {
            return;
        }
        let step = u64::from(PREVIEW_ROWS);
        let mut shape = self.preview_controls.applied.clone();
        shape.offset = if forward {
            shape.offset.saturating_add(step)
        } else {
            shape.offset.saturating_sub(step)
        };
        self.reshape_preview(shape, cx);
    }

    /// Unfolds the sort menu under the bar, as `Columns` unfolds its manager.
    pub(in crate::workspace) fn toggle_preview_sort(&mut self, cx: &mut Context<'_, Self>) {
        if !self.preview_sort_available() {
            return;
        }
        self.preview_controls.sort_open = !self.preview_controls.sort_open;
        cx.notify();
    }

    /// Puts the menu back on the sort the rows on screen came from.
    ///
    /// Called when a read ends without producing rows: the menu had already
    /// committed the rank the user accepted, and leaving it there would name an
    /// order nothing on screen has.
    pub(in crate::workspace) fn resync_preview_sort(&mut self, cx: &mut Context<'_, Self>) {
        let rank = sort_rank(
            &self.preview_controls.sort_choices,
            &self.preview_controls.applied,
        );
        self.preview_controls
            .sort
            .update(cx, |field, cx| field.set_selected(rank, cx));
    }
}
