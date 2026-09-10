//! Workspace sidebar rendering using real session state.

use super::layout::Control;
use super::*;
use gpui::{AnyElement, FontWeight, div, px};
use oxyn_ui::icons::{IconName, icon, logo};
use oxyn_ui::{Theme, environment_label};

impl Workspace {
    pub(super) fn sidebar(&self, compact: bool, cx: &Context<'_, Self>) -> AnyElement {
        let theme = Theme::of(cx);
        div()
            .w(px(if compact { 64. } else { 280. }))
            .h_full()
            .flex_none()
            .flex()
            .flex_col()
            .p_3()
            .when(compact, |el| el.items_center())
            .gap_6()
            .overflow_hidden()
            .child(
                div()
                    .h(px(48.))
                    .flex_none()
                    .flex()
                    .items_center()
                    .gap_2()
                    .px_1()
                    .child(logo(theme.mode))
                    .when(!compact, |el| {
                        el.child(
                            div()
                                .flex()
                                .flex_col()
                                .child(div().font_weight(FontWeight::MEDIUM).child("Oxyn"))
                                .child(
                                    div()
                                        .text_size(theme.typography.small_size)
                                        .text_color(theme.colors.text_muted)
                                        .child("Personal workspace"),
                                ),
                        )
                    }),
            )
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap_1()
                    .when(self.capabilities.contains(Capabilities::SQL), |el| {
                        el.child(self.control(
                            "nav-sql",
                            "SQL editor · ⌘J",
                            Control::Sql,
                            compact,
                            cx,
                        ))
                    })
                    .child(self.control(
                        "nav-library",
                        "History & queries · ⌘⇧H",
                        Control::Library,
                        compact,
                        cx,
                    ))
                    .child(self.control(
                        "nav-catalog",
                        "Connections · ⌘1",
                        Control::Catalog,
                        compact,
                        cx,
                    )),
            )
            .when(!compact, |el| {
                el.child(
                    div()
                        .flex_1()
                        .min_h_0()
                        .flex()
                        .flex_col()
                        .gap_2()
                        .child(
                            div()
                                .flex()
                                .items_center()
                                .justify_between()
                                .child(
                                    div()
                                        .text_size(theme.typography.small_size)
                                        .text_color(theme.colors.text_muted)
                                        .child("CONNECTIONS"),
                                )
                                .child(self.control(
                                    "new-connection",
                                    "New connection",
                                    Control::NewConnection,
                                    true,
                                    cx,
                                )),
                        )
                        .child(
                            div()
                                .flex()
                                .items_center()
                                .gap_2()
                                .h(px(32.))
                                .px_2()
                                .child(
                                    icon(IconName::Database)
                                        .size(px(16.))
                                        .text_color(theme.colors.text_muted),
                                )
                                .child(
                                    div()
                                        .flex_1()
                                        .min_w_0()
                                        .truncate()
                                        .child(self.display.name.clone()),
                                )
                                .child(
                                    div()
                                        .text_size(theme.typography.small_size)
                                        .text_color(theme.colors.environment(self.environment))
                                        .child(environment_label(self.environment)),
                                ),
                        )
                        .child(self.catalog_panel(cx)),
                )
            })
            .when(compact, |el| {
                el.child(div().flex_1()).child(self.control(
                    "new-connection-compact",
                    "New connection",
                    Control::NewConnection,
                    true,
                    cx,
                ))
            })
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap_2()
                    .flex_none()
                    .child(self.control(
                        "workspace-help",
                        "Workspace guide",
                        Control::Help,
                        compact,
                        cx,
                    ))
                    .child(self.control(
                        "workspace-theme",
                        "Settings · ⌘,",
                        Control::Preferences,
                        compact,
                        cx,
                    ))
                    .when(!compact, |el| {
                        el.child(
                            div()
                                .text_size(theme.typography.small_size)
                                .text_color(theme.colors.text_muted)
                                .child("Connected · Current session"),
                        )
                    }),
            )
            .into_any_element()
    }

    fn catalog_panel(&self, cx: &Context<'_, Self>) -> AnyElement {
        let theme = Theme::of(cx);
        if !self.catalog_supported() {
            return div()
                .text_size(theme.typography.small_size)
                .text_color(theme.colors.text_muted)
                .child("This session does not expose a catalog.")
                .into_any_element();
        }
        let (title, detail) = self.catalog_state.message();
        div()
            .flex_1()
            .min_h_0()
            .flex()
            .flex_col()
            .gap_2()
            .child(
                div()
                    .flex()
                    .items_center()
                    .justify_between()
                    .child(
                        div()
                            .text_size(theme.typography.small_size)
                            .text_color(theme.colors.text_muted)
                            .child(title),
                    )
                    .child(if self.catalog_active.is_some() {
                        self.control(
                            "catalog-cancel",
                            "Cancel catalog load",
                            Control::CancelCatalog,
                            false,
                            cx,
                        )
                    } else {
                        self.control("catalog-refresh", "Refresh", Control::Refresh, false, cx)
                    }),
            )
            .when(!detail.is_empty(), |el| {
                el.child(
                    div()
                        .px_2()
                        .text_size(theme.typography.small_size)
                        .text_color(if matches!(self.catalog_state, CatalogState::Error(_)) {
                            theme.colors.danger
                        } else {
                            theme.colors.text_muted
                        })
                        .child(detail),
                )
            })
            .when_some(
                self.catalog.clone().filter(|_| {
                    matches!(
                        self.catalog_state,
                        CatalogState::Ready
                            | CatalogState::Empty
                            | CatalogState::Cancelled
                            | CatalogState::Error(_)
                    )
                }),
                |el, tree| el.child(div().flex_1().min_h_0().child(tree)),
            )
            .into_any_element()
    }
}
