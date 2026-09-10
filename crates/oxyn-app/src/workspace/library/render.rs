//! Figma 47:7638: paged rows above the selected query, with labeled filters.

use super::*;

impl QueryLibrary {
    fn list_panel(&self, cx: &Context<'_, Self>) -> AnyElement {
        let theme = Theme::of(cx);
        div()
            .flex_1()
            .min_w_0()
            .min_h_0()
            .flex()
            .flex_col()
            .gap_2()
            .child(
                div()
                    .h(px(40.))
                    .text_size(theme.typography.small_size)
                    .text_color(theme.colors.text_muted)
                    .items_center()
                    .flex_none()
                    .flex()
                    .gap_2()
                    .px_2()
                    .bg(theme.colors.surface)
                    .child(div().flex_1().child("Query"))
                    .child(div().w(px(110.)).child("Connection"))
                    .child(div().w(px(110.)).child("Status"))
                    .child(div().w(px(60.)).child("Duration"))
                    .child(div().w(px(128.)).child("When · UTC")),
            )
            .child(
                uniform_list(
                    "library-rows",
                    self.rows.len(),
                    cx.processor(|this, range: std::ops::Range<usize>, _, cx| {
                        let theme = Theme::of(cx);
                        range
                            .filter_map(|index| {
                                this.rows.get(index).map(|row| {
                                    div()
                                        .id(("library-row", index))
                                        .h(px(40.))
                                        .flex()
                                        .items_center()
                                        .gap_2()
                                        .px_2()
                                        .border_b_1()
                                        .border_color(theme.colors.border)
                                        .bg(if this.selected == Some(index) {
                                            theme.colors.hover
                                        } else {
                                            theme.colors.background
                                        })
                                        .cursor_pointer()
                                        .on_click(cx.listener(move |this, _, window, cx| {
                                            window.focus(&this.focus);
                                            this.inspect(index, cx);
                                        }))
                                        .child(
                                            div()
                                                .flex_1()
                                                .min_w_0()
                                                .truncate()
                                                .child(row.title.clone()),
                                        )
                                        .child(
                                            div()
                                                .w(px(110.))
                                                .truncate()
                                                .child(row.connection.clone()),
                                        )
                                        .child(
                                            div().w(px(110.)).truncate().child(row.status.clone()),
                                        )
                                        .child(
                                            div().w(px(60.)).truncate().child(row.duration.clone()),
                                        )
                                        .child(div().w(px(128.)).truncate().child(row.when.clone()))
                                })
                            })
                            .collect::<Vec<_>>()
                    }),
                )
                .flex_1()
                .min_h(px(80.))
                .track_scroll(self.scroll.clone())
                .track_focus(&self.focus)
                .tab_index(0)
                .border_1()
                .border_color(theme.colors.border)
                .focus(|el| el.border_color(theme.colors.border_focus))
                .on_key_down(cx.listener(|this, event, _, cx| this.list_key(event, cx))),
            )
            .child(
                div()
                    .flex_none()
                    .flex()
                    .flex_wrap()
                    .items_center()
                    .gap_2()
                    .child(div().flex_1().child(self.notice.clone()))
                    .when(self.cursors.len() > 1 && self.request.is_none(), |el| {
                        el.child(self.button(
                            "library-previous",
                            "Previous page",
                            Action::Previous,
                            cx,
                        ))
                    })
                    .when(self.next.is_some() && self.request.is_none(), |el| {
                        el.child(self.button("library-next", "Next page", Action::Next, cx))
                    }),
            )
            .into_any_element()
    }
    fn detail_panel(&self, cx: &Context<'_, Self>) -> AnyElement {
        let theme = Theme::of(cx);
        div().flex_none().flex().flex_col().gap_3().p_4().border_1().border_color(theme.colors.border).rounded(px(8.))
            .h(px(238.))
            .child(div().child(match &self.detail {
                Some(Detail::Document(document)) => format!("Selected query · {}", document.saved_title.as_ref().unwrap_or(&document.title)),
                Some(Detail::History(_)) => "Selected execution".into(),
                None => "Query details".into(),
            }).truncate().font_weight(gpui::FontWeight::SEMIBOLD))
            .child(div().text_color(theme.colors.text_muted).child(self.detail_notice.clone()))
            .child(div().flex().flex_wrap().gap_2()
                .when(self.detail.is_some(), |el| el.child(self.button("library-copy", "Copy query", Action::Copy, cx)))
                .when(self.can_open_result(), |el| el.child(self.button("library-open-result", "Open retained result", Action::OpenResult, cx)))
                .when(self.can_open_copy(), |el| el.child(self.button("library-open-copy", format!("Open copy on {}", self.current.as_ref().map_or("current connection", |current| current.1.as_str())), Action::OpenCopy, cx)))
                .when(self.can_edit_original(), |el| el.child(self.button("library-edit-original", "Resume working query", Action::EditOriginal, cx)))
                .when(matches!(&self.detail, Some(Detail::Document(doc)) if doc.saved_content.as_deref() != Some(&doc.content)), |el| el.child(self.button("library-working", if self.working_copy { "Show saved copy" } else { "Inspect working copy" }, Action::WorkingCopy, cx))))
            .child(div().flex_1().min_h_0().child(self.reader.clone())).into_any_element()
    }
}
impl Render for QueryLibrary {
    fn render(&mut self, window: &mut Window, cx: &mut Context<'_, Self>) -> impl IntoElement {
        if self.show_retained {
            if self.focus_retained
                && let Some(view) = &self.retained
            {
                self.focus_retained = false;
                window.focus(&view.read(cx).grid.read(cx).focus_handle(cx));
            }
            return self.retained_view(cx);
        }
        if self.focus_library {
            self.focus_library = false;
            window.focus(&self.focus);
        }
        let theme = Theme::of(cx);

        let loading = self.request.is_some()
            || self.detail_request.is_some()
            || self.result_request.is_some()
            || self.search_pending;
        div().flex_1().min_h_0().min_w_0().flex().flex_col().gap_3().p_3().bg(theme.colors.background)
            .text_color(theme.colors.text).text_size(theme.typography.ui_size)
            .child(div().flex_none().flex().flex_wrap().gap_2()
                .child(self.button("library-history", "History", Action::Tab(Tab::History), cx))
                .child(self.button("library-saved", "Saved queries", Action::Tab(Tab::Saved), cx))
                .child(self.button("library-results", "Recent results", Action::Tab(Tab::Results), cx))
                .when(self.retained.is_some(), |el| el.child(self.button("library-current-result", "Opened result", Action::ShowRetained, cx))))
            .child(div().flex_none().flex().flex_wrap().items_center().gap_2()
                .child(div().w(px(300.)).h(px(74.)).flex().flex_col().gap_2()
                    .child(div().text_size(theme.typography.small_size).text_color(theme.colors.text_muted).child("Search"))
                    .child(self.search.clone()))
                .when(self.tab != Tab::Saved, |el| el.child(div().h(px(74.)).flex().flex_col().gap_2()
                        .child(div().text_size(theme.typography.small_size).text_color(theme.colors.text_muted).child("Connection"))
                        .child(div().w(px(200.)).child(self.connection_select.clone())))
                    .child(div().h(px(74.)).flex().flex_col().gap_2()
                        .child(div().text_size(theme.typography.small_size).text_color(theme.colors.text_muted).child("Period and status"))
                        .child(div().flex().gap_2().child(div().w(px(150.)).child(self.days_select.clone())).child(div().w(px(150.)).child(self.status_select.clone())))))
                .child(self.button("library-refresh", "Refresh", Action::Refresh, cx))
                .when(loading, |el| el.child(self.button("library-cancel", "Cancel loading", Action::Cancel, cx))))
            .child(div().flex_none().text_size(theme.typography.small_size).text_color(theme.colors.text_muted)
                .child(if self.tab == Tab::Saved { "Search saved titles and queries · this workspace" } else if self.tab == Tab::Results { "Recorded results · buffers from previous runs may have expired · local history across workspaces" } else { "Search query text · local history across workspaces · ambiguous writes are never replayed" }))
            .child(div().flex_1().min_h_0().flex().gap_3().flex_col()
                .child(self.list_panel(cx)).child(self.detail_panel(cx))).into_any_element()
    }
}
