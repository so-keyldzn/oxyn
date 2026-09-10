//! Correlated, cancellable page requests. This module never reads a file.

use super::*;

#[derive(Debug, Default)]
pub(super) struct PageState {
    generation: u64,
    pub(super) pending: Option<BatchIndex>,
    error: Option<SharedString>,
}

impl DataGrid {
    pub(super) fn invalidate_pages(&mut self, cx: &mut Context<'_, Self>) {
        if self.pages.pending.take().is_some() {
            cx.emit(GridEvent::CancelPageRequested {
                generation: self.pages.generation,
            });
        }
        self.pages.generation = self.pages.generation.wrapping_add(1);
        self.pages.error = None;
    }

    pub(super) fn request_page(&mut self, batch: BatchIndex, cx: &mut Context<'_, Self>) {
        if self.pages.pending.is_some() || self.pages.error.is_some() {
            return;
        }
        self.pages.pending = Some(batch);
        cx.emit(GridEvent::PageRequested {
            generation: self.pages.generation,
            batch: batch.get(),
        });
    }

    /// Completes the matching local read only; a previous result cannot change this grid.
    pub fn complete_page(
        &mut self,
        generation: u64,
        batch: usize,
        error: Option<String>,
        cx: &mut Context<'_, Self>,
    ) {
        if generation != self.pages.generation || self.pages.pending != Some(BatchIndex::new(batch))
        {
            return;
        }
        self.pages.pending = None;
        self.pages.error = error.map(SharedString::from);
        self.on_batch(cx);
    }

    pub(super) fn cancel_page_load(&mut self, cx: &mut Context<'_, Self>) {
        if self.pages.pending.take().is_some() {
            cx.emit(GridEvent::CancelPageRequested {
                generation: self.pages.generation,
            });
            self.pages.error =
                Some("Local page loading cancelled. The query was not re-executed.".into());
            cx.notify();
        }
    }

    pub(super) fn render_page_status(&self, cx: &Context<'_, Self>) -> AnyElement {
        let theme = Theme::of(cx);
        let base = div()
            .flex_none()
            .flex()
            .items_center()
            .gap_2()
            .px_3()
            .py_1()
            .text_size(theme.typography.small_size)
            .text_color(theme.colors.text_muted);
        if self.pages.pending.is_some() {
            base.child("Loading existing result page…")
                .child(
                    div()
                        .id("cancel-result-page")
                        .tab_index(0)
                        .px_2()
                        .py_1()
                        .border_1()
                        .border_color(theme.colors.border)
                        .rounded(theme.radii.control)
                        .focus(|style| style.border_color(theme.colors.border_focus))
                        .cursor_pointer()
                        .child("Cancel page loading")
                        .on_click(cx.listener(|this, _, _, cx| this.cancel_page_load(cx)))
                        .on_key_down(cx.listener(|this, event: &KeyDownEvent, _, cx| {
                            if matches!(event.keystroke.key.as_str(), "enter" | "space") {
                                this.cancel_page_load(cx);
                                cx.stop_propagation();
                            }
                        })),
                )
                .into_any_element()
        } else if let Some(error) = &self.pages.error {
            base.child(
                div()
                    .flex_1()
                    .min_w_0()
                    .text_color(theme.colors.danger)
                    .child(error.clone()),
            )
            .child(
                div()
                    .id("retry-result-page")
                    .tab_index(0)
                    .px_2()
                    .py_1()
                    .border_1()
                    .border_color(theme.colors.border)
                    .rounded(theme.radii.control)
                    .focus(|style| style.border_color(theme.colors.border_focus))
                    .cursor_pointer()
                    .child("Retry local read")
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.pages.error = None;
                        this.pages.generation = this.pages.generation.wrapping_add(1);
                        cx.notify();
                    }))
                    .on_key_down(cx.listener(|this, event: &KeyDownEvent, _, cx| {
                        if matches!(event.keystroke.key.as_str(), "enter" | "space") {
                            this.pages.error = None;
                            this.pages.generation = this.pages.generation.wrapping_add(1);
                            cx.notify();
                            cx.stop_propagation();
                        }
                    })),
            )
            .into_any_element()
        } else {
            div().into_any_element()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[gpui::test]
    fn stale_page_completion_and_failure_cannot_change_a_new_result(cx: &mut gpui::TestAppContext) {
        let (grid, cx) = cx.add_window_view(|_, cx| DataGrid::new(cx));
        grid.update(cx, |grid, cx| {
            grid.request_page(BatchIndex::new(8), cx);
            let old = grid.pages.generation;
            grid.reset(cx);
            grid.request_page(BatchIndex::new(2), cx);
            grid.complete_page(old, 8, Some("obsolete failure".into()), cx);
            assert_eq!(grid.pages.pending, Some(BatchIndex::new(2)));
            assert!(grid.pages.error.is_none());
            grid.complete_page(
                grid.pages.generation,
                2,
                Some("disk read failed".into()),
                cx,
            );
            assert!(grid.pages.pending.is_none());
            grid.request_page(BatchIndex::new(3), cx);
            assert!(
                grid.pages.pending.is_none(),
                "a failed read must not retry automatically"
            );
            assert_eq!(
                grid.pages.error.as_ref().map(ToString::to_string),
                Some("disk read failed".into())
            );
        });
    }
}
