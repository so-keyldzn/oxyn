//! Drawing the preview bar of Figma `190:1618`, and the words under it.
//!
//! Separated from the state next door because these are two subjects: what the
//! preview asks of the relation, and what the toolbar shows of it. Being a
//! child module keeps the access to the workspace's private fields the drawing
//! needs.

use super::{Pagination, PreviewState};
use crate::workspace::layout::Control;
use crate::workspace::preview::PREVIEW_ROWS;
use crate::workspace::{Context, Workspace};
use gpui::prelude::*;
use gpui::{AnyElement, div, px};
use oxyn_ui::Theme;

impl Workspace {
    /// The filter and sort bar, absent when this session can act on neither.
    ///
    /// Figma `190:1618`: 1272 × 32 under the Data bar, a field of 1180 px
    /// prefixed with the literal `WHERE`, then `Sort` in 84 px at x = 1188.
    pub(in crate::workspace) fn preview_filter_bar(
        &self,
        cx: &Context<'_, Self>,
    ) -> Option<AnyElement> {
        if !self.preview_bar_available() {
            return None;
        }
        let theme = Theme::of(cx);
        Some(
            div()
                .id("preview-filter-bar")
                .debug_selector(|| "preview-filter-bar".into())
                .h(px(32.))
                .flex_none()
                .flex()
                .items_center()
                .gap_2()
                .when(self.preview_filter_available(), |el| {
                    el.child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .flex()
                            .items_center()
                            .gap_2()
                            .child(
                                // The literal keyword of the mock: what the
                                // field holds is the predicate, not the clause.
                                div()
                                    .flex_none()
                                    .text_color(theme.colors.text_muted)
                                    .child("WHERE"),
                            )
                            .child(
                                div()
                                    .id("preview-filter-input")
                                    .debug_selector(|| "preview-filter-input".into())
                                    .flex_1()
                                    .min_w_0()
                                    .child(self.preview_controls.predicate.clone()),
                            )
                            .child(self.control(
                                "preview-filter-apply",
                                "Apply",
                                Control::PreviewApply,
                                false,
                                cx,
                            )),
                    )
                })
                .when(self.preview_sort_available(), |el| {
                    el.child(self.control("preview-sort", "Sort", Control::PreviewSort, false, cx))
                })
                .into_any_element(),
        )
    }

    /// The sort menu, unfolded under the bar as `Columns` unfolds its manager.
    ///
    /// The 84 px of Figma `190:1618` hold a button and nothing else; a menu
    /// pushed into them would be a menu nobody can read. Unfolding it below is
    /// the shape this toolbar already uses for its column manager.
    pub(in crate::workspace) fn preview_sort_panel(
        &self,
        cx: &Context<'_, Self>,
    ) -> Option<AnyElement> {
        if !self.preview_sort_available() || !self.preview_controls.sort_open {
            return None;
        }
        let theme = Theme::of(cx);
        let described = self.preview_controls.sort_choices.len() > 1;
        Some(
            div()
                .id("preview-sort-panel")
                .debug_selector(|| "preview-sort-panel".into())
                .flex_none()
                .flex()
                .items_center()
                .gap_3()
                .p_3()
                .border_1()
                .border_color(theme.colors.border)
                .rounded(theme.radii.surface)
                .child(
                    div()
                        .flex_none()
                        .text_color(theme.colors.text_muted)
                        .child("Order the server returns rows in"),
                )
                .when(described, |el| {
                    el.child(
                        div()
                            .id("preview-sort-field")
                            .debug_selector(|| "preview-sort-field".into())
                            .w(px(260.))
                            .flex_none()
                            .child(self.preview_controls.sort.clone()),
                    )
                })
                .when(!described, |el| {
                    // The empty state of this menu, and the way out of it: the
                    // columns are read from the catalog already in memory, so
                    // an undescribed relation has nothing to offer until the
                    // catalog is asked ([I-05](../../../../CLAUDE.md#i-05)).
                    el.child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .text_size(theme.typography.small_size)
                            .text_color(theme.colors.text_muted)
                            .child(
                                "The columns of this relation have not been read yet. \
                                 Load them to choose an order.",
                            ),
                    )
                    .child(self.control(
                        "preview-sort-load",
                        "Load columns",
                        Control::PreviewColumns,
                        false,
                        cx,
                    ))
                })
                .into_any_element(),
        )
    }

    /// What the bar cannot say in its own width: the state, in words.
    ///
    /// Returns `None` only when there is nothing to add to the rows on screen.
    pub(in crate::workspace) fn preview_shape_notice(
        &self,
        cx: &Context<'_, Self>,
    ) -> Option<AnyElement> {
        if !self.preview_bar_available() {
            return None;
        }
        let theme = Theme::of(cx);
        let applied = self.preview_controls.applied.predicate();
        let draft = self.preview_controls.predicate.read(cx).text().to_owned();
        let pending = self.preview_filter_available() && super::draft_differs(&draft, applied);
        let (message, failed) = match self.preview_state(cx) {
            // The server's own words, code included: the audience reads them,
            // and the grid next door already shows them. What is added here is
            // what the server does not say — that nothing on screen changed.
            PreviewState::Failed { .. } => (
                "This read was refused. The rows of the previous shape were not kept under \
                 the new one, and the text stays where it can be fixed."
                    .to_owned(),
                true,
            ),
            PreviewState::Empty { filtered: true } => (
                "No row matches this filter. The read succeeded; the table may still hold rows."
                    .to_owned(),
                false,
            ),
            PreviewState::Empty { filtered: false } => {
                ("This table holds no rows.".to_owned(), false)
            }
            PreviewState::Loading if pending => (
                "Reading… This filter is not in force until the server answers.".to_owned(),
                false,
            ),
            PreviewState::Loading => return None,
            PreviewState::Initial | PreviewState::Loaded { .. } if pending => (
                "This filter is not applied yet. Apply runs a new read against the server; \
                 the rows below still come from the shape in force."
                    .to_owned(),
                false,
            ),
            PreviewState::Initial | PreviewState::Loaded { .. } => {
                match self.preview_pagination(cx) {
                    // Saying nothing here would be saying the preview is the
                    // table; it is its first page, in no guaranteed order.
                    Pagination::NeedsOrder => (
                        "Rows come in no guaranteed order. Sort the preview to browse it page \
                         by page: an OFFSET over an uncertain order shows a row twice and hides \
                         another, with nothing to say so."
                            .to_owned(),
                        false,
                    ),
                    Pagination::NoUniqueKey => (
                        "This preview stays on its first page: no unique key is known for this \
                         relation, so no order can be made total."
                            .to_owned(),
                        false,
                    ),
                    Pagination::Ready { previous, .. } if previous => (
                        // Each page is an execution, and the ADR asks that this
                        // be said rather than left to look like a snapshot.
                        format!(
                            "Rows {}–{} of this order. Each page is a new read: the server may \
                             have changed between two of them.",
                            self.preview_controls.applied.offset.saturating_add(1),
                            self.preview_controls
                                .applied
                                .offset
                                .saturating_add(u64::from(PREVIEW_ROWS)),
                        ),
                        false,
                    ),
                    Pagination::Ready { .. } => return None,
                }
            }
        };
        Some(
            div()
                .id("preview-shape-notice")
                .debug_selector(|| "preview-shape-notice".into())
                .flex_none()
                .min_w_0()
                .text_size(theme.typography.small_size)
                .text_color(if failed {
                    theme.colors.warning
                } else {
                    theme.colors.text_muted
                })
                .child(message)
                .into_any_element(),
        )
    }

    /// The page controls, on the status line the mock leaves room in.
    ///
    /// Figma draws no page control at all (`190:1860` counts rows and nothing
    /// else); this is the deviation recorded in
    /// [FIGMA-HANDOFF](../../../../docs/FIGMA-HANDOFF.md). They sit next to the
    /// count because that is the sentence that explains them.
    pub(in crate::workspace) fn preview_pages(&self, cx: &Context<'_, Self>) -> Option<AnyElement> {
        if !self.preview_bar_available() {
            return None;
        }
        // Offered only over rows that are actually on screen: a page turned
        // while a read is in flight would be a control that does nothing.
        if !matches!(
            self.preview_state(cx),
            PreviewState::Loaded { .. } | PreviewState::Empty { .. }
        ) {
            return None;
        }
        let Pagination::Ready { previous, next } = self.preview_pagination(cx) else {
            return None;
        };
        if !previous && !next {
            return None;
        }
        Some(
            div()
                .flex_none()
                .flex()
                .items_center()
                .gap_2()
                .when(previous, |el| {
                    el.child(self.control(
                        "preview-page-previous",
                        "Previous page",
                        Control::PreviewPreviousPage,
                        false,
                        cx,
                    ))
                })
                .when(next, |el| {
                    el.child(self.control(
                        "preview-page-next",
                        "Next page",
                        Control::PreviewNextPage,
                        false,
                        cx,
                    ))
                })
                .into_any_element(),
        )
    }
}
