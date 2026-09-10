//! Capability-aware metadata tabs, reading only visible rows from the shared cache.

use super::layout::Control;
use super::*;
use gpui::{AnyElement, ClipboardItem, KeyDownEvent, ScrollStrategy, div, px, uniform_list};
use oxyn_catalog::CatalogCache;
use oxyn_ui::Theme;

fn required_capability(tab: ObjectTab) -> Capabilities {
    match tab {
        ObjectTab::Indexes => Capabilities::INDEXES,
        ObjectTab::Constraints => Capabilities::CONSTRAINTS,
        ObjectTab::Relations => Capabilities::FOREIGN_KEYS,
        ObjectTab::IncomingRelations => Capabilities::INCOMING_FOREIGN_KEYS,
        ObjectTab::Ddl => Capabilities::OBJECT_DEFINITION,
        _ => Capabilities::empty(),
    }
}

fn metadata_count(cache: &CatalogCache, path: &CatalogPath, tab: ObjectTab) -> Option<usize> {
    match tab {
        ObjectTab::Constraints => cache.constraints(path).map(<[_]>::len),
        ObjectTab::Indexes => cache.indexes(path).map(<[_]>::len),
        ObjectTab::Relations => cache.foreign_keys(path).map(<[_]>::len),
        ObjectTab::IncomingRelations => cache.incoming_foreign_keys(path).map(<[_]>::len),
        ObjectTab::Structure => cache.relation(path).map(|relation| relation.fields.len()),
        ObjectTab::Data | ObjectTab::Ddl => None,
    }
}

fn metadata_row(
    cache: &CatalogCache,
    path: &CatalogPath,
    tab: ObjectTab,
    index: usize,
) -> Option<[String; 4]> {
    match tab {
        ObjectTab::Constraints => {
            let constraint = cache.constraints(path)?.get(index)?;
            Some([
                if constraint.name.is_empty() {
                    "Unnamed".into()
                } else {
                    constraint.name.clone()
                },
                constraint.kind.as_str().replace('_', " ").to_uppercase(),
                if constraint.fields.is_empty() {
                    "Not reported".into()
                } else {
                    constraint.fields.join(", ")
                },
                match constraint.validated {
                    Some(true) => "Validated",
                    Some(false) => "Not validated",
                    None => "Not reported",
                }
                .into(),
            ])
        }
        ObjectTab::IncomingRelations => {
            let incoming = cache.incoming_foreign_keys(path)?.get(index)?;
            Some([
                format!("{} ({})", incoming.source, incoming.key.fields.join(", ")),
                format!(
                    "{} ({})",
                    incoming.key.references.relation,
                    incoming.key.references.fields.join(", ")
                ),
                match incoming.source_unique {
                    Some(true) => "One to one",
                    Some(false) => "Many to one",
                    None => "Not reported",
                }
                .into(),
                String::new(),
            ])
        }
        ObjectTab::Indexes => {
            let index = cache.indexes(path)?.get(index)?;
            Some([
                index.name.clone(),
                index.fields.join(", "),
                if index.unique { "Unique" } else { "Non-unique" }.into(),
                index
                    .method
                    .clone()
                    .unwrap_or_else(|| "Not reported".into()),
            ])
        }
        ObjectTab::Relations => {
            let key = cache.foreign_keys(path)?.get(index)?;
            Some([
                if key.name.is_empty() {
                    "Unnamed".into()
                } else {
                    key.name.clone()
                },
                key.fields.join(", "),
                format!(
                    "{} ({})",
                    key.references.relation,
                    key.references.fields.join(", ")
                ),
                key.on_delete.as_str().into(),
            ])
        }
        _ => None,
    }
}

fn metadata_cell(tab: ObjectTab, column: usize) -> gpui::Div {
    let fraction = if tab == ObjectTab::Constraints {
        [280. / 840., 220. / 840., 200. / 840., 140. / 840.]
            .get(column)
            .copied()
            .unwrap_or(0.25)
    } else if tab == ObjectTab::IncomingRelations {
        [340. / 840., 300. / 840., 200. / 840., 0.]
            .get(column)
            .copied()
            .unwrap_or(0.)
    } else {
        0.25
    };
    div()
        .flex_none()
        .w(gpui::relative(fraction))
        .min_w_0()
        .px_2()
        .truncate()
}

impl Workspace {
    fn related_object(&self) -> Option<CatalogPath> {
        let path = self.selected_path.as_ref()?;
        let cache = self.catalog_cache.try_read()?;
        match self.object_tab {
            ObjectTab::IncomingRelations => Some(
                cache
                    .incoming_foreign_keys(path)?
                    .get(self.metadata_selected)?
                    .source
                    .clone(),
            ),
            ObjectTab::Relations => Some(
                cache
                    .foreign_keys(path)?
                    .get(self.metadata_selected)?
                    .references
                    .relation
                    .clone(),
            ),
            _ => None,
        }
    }

    pub(super) fn open_related_object(&mut self, cx: &mut Context<'_, Self>) {
        if let Some(path) = self.related_object() {
            self.select_object(path.clone(), cx);
            if self
                .catalog_cache
                .try_read()
                .is_some_and(|cache| cache.relation_summary(&path).is_none())
            {
                self.refresh_catalog(CatalogScope::Relation(path), cx);
            }
        }
    }

    fn selected_constraint_definition(&self) -> Option<String> {
        if self.object_tab != ObjectTab::Constraints {
            return None;
        }
        let cache = self.catalog_cache.try_read()?;
        cache
            .constraints(self.selected_path.as_ref()?)?
            .get(self.metadata_selected)?
            .expression
            .clone()
    }

    fn copy_constraint_definition(&self, cx: &mut Context<'_, Self>) {
        if let Some(text) = self.selected_constraint_definition() {
            cx.write_to_clipboard(ClipboardItem::new_string(text));
        }
    }

    pub(super) fn on_metadata_key(
        &mut self,
        event: &KeyDownEvent,
        _: &mut Window,
        cx: &mut Context<'_, Self>,
    ) {
        if event.keystroke.key == "enter"
            && matches!(
                self.object_tab,
                ObjectTab::Relations | ObjectTab::IncomingRelations
            )
        {
            self.open_related_object(cx);
            cx.stop_propagation();
            return;
        }
        if event.keystroke.key == "c"
            && (event.keystroke.modifiers.platform || event.keystroke.modifiers.control)
        {
            self.copy_constraint_definition(cx);
            cx.stop_propagation();
            return;
        }
        let count = self
            .selected_path
            .as_ref()
            .and_then(|path| {
                let cache = self.catalog_cache.try_read()?;
                metadata_count(&cache, path, self.object_tab)
            })
            .unwrap_or(0);
        let last = count.saturating_sub(1);
        let selected = match event.keystroke.key.as_str() {
            "up" => self.metadata_selected.saturating_sub(1),
            "down" => self.metadata_selected.saturating_add(1).min(last),
            "pageup" => self.metadata_selected.saturating_sub(10),
            "pagedown" => self.metadata_selected.saturating_add(10).min(last),
            "home" => 0,
            "end" => last,
            _ => return,
        };
        self.metadata_selected = selected.min(last);
        self.metadata_scroll
            .scroll_to_item(self.metadata_selected, ScrollStrategy::Center);
        cx.notify();
        cx.stop_propagation();
    }

    pub(super) fn select_metadata_tab(&mut self, tab: ObjectTab, cx: &mut Context<'_, Self>) {
        self.object_tab = tab;
        self.metadata_selected = 0;
        self.metadata_scroll.scroll_to_item(0, ScrollStrategy::Top);
        self.metadata_menu_open = false;
        self.cancel_preview(cx);
        self.ensure_definition(cx);
        if tab == ObjectTab::Ddl {
            cx.notify();
            return;
        }
        if !self.capabilities.contains(required_capability(tab)) {
            cx.notify();
            return;
        }
        let loaded = self.selected_path.as_ref().is_some_and(|path| {
            self.catalog_cache.try_read().is_some_and(|cache| {
                metadata_count(&cache, path, tab).is_some()
                    && match tab {
                        ObjectTab::Constraints => !matches!(
                            cache.freshness(&CatalogScope::Constraints(path.clone())),
                            oxyn_catalog::Freshness::Invalidated
                        ),
                        ObjectTab::IncomingRelations => !matches!(
                            cache.freshness(&CatalogScope::IncomingForeignKeys(path.clone())),
                            oxyn_catalog::Freshness::Invalidated
                        ),
                        _ => true,
                    }
            })
        });
        if !loaded {
            self.refresh_object_metadata(cx);
        }
        cx.notify();
    }

    pub(super) fn refresh_object_metadata(&mut self, cx: &mut Context<'_, Self>) {
        if !self
            .capabilities
            .contains(required_capability(self.object_tab))
        {
            return;
        }
        if let Some(path) = self
            .selected_path
            .clone()
            .filter(|path| path.relation().is_some())
        {
            let scope = match self.object_tab {
                ObjectTab::Constraints => CatalogScope::Constraints(path),
                ObjectTab::IncomingRelations => CatalogScope::IncomingForeignKeys(path),
                _ => CatalogScope::Relation(path),
            };
            self.refresh_catalog(scope, cx);
        }
    }

    pub(super) fn object_tabs(&self, cx: &Context<'_, Self>) -> AnyElement {
        div()
            .flex()
            .flex_col()
            .gap_1()
            .flex_none()
            .child(
                div()
                    .flex()
                    .gap_1()
                    .child(self.control("object-data", "Data", Control::Data, false, cx))
                    .child(self.control(
                        "object-structure",
                        "Structure",
                        Control::Describe,
                        false,
                        cx,
                    ))
                    .when(!self.compact_layout, |el| {
                        el.child(self.control(
                            "object-indexes",
                            "Indexes",
                            Control::Indexes,
                            false,
                            cx,
                        ))
                        .child(self.control(
                            "object-constraints",
                            "Constraints",
                            Control::Constraints,
                            false,
                            cx,
                        ))
                        .child(self.control(
                            "object-relations",
                            "Relations",
                            Control::Relations,
                            false,
                            cx,
                        ))
                        .child(self.control(
                            "object-ddl",
                            "DDL",
                            Control::Definition,
                            false,
                            cx,
                        ))
                    })
                    .when(self.compact_layout, |el| {
                        el.child(self.control(
                            "object-more",
                            "More",
                            Control::MoreMetadata,
                            false,
                            cx,
                        ))
                    }),
            )
            .when(self.compact_layout && self.metadata_menu_open, |el| {
                el.child(
                    div()
                        .flex()
                        .gap_1()
                        .child(self.control(
                            "object-indexes-menu",
                            "Indexes",
                            Control::Indexes,
                            false,
                            cx,
                        ))
                        .child(self.control(
                            "object-constraints-menu",
                            "Constraints",
                            Control::Constraints,
                            false,
                            cx,
                        ))
                        .child(self.control(
                            "object-relations-menu",
                            "Relations",
                            Control::Relations,
                            false,
                            cx,
                        ))
                        .child(self.control(
                            "object-ddl-menu",
                            "DDL",
                            Control::Definition,
                            false,
                            cx,
                        )),
                )
            })
            .into_any_element()
    }

    pub(super) fn object_metadata(&self, cx: &Context<'_, Self>) -> AnyElement {
        div()
            .flex_1()
            .min_h_0()
            .flex()
            .flex_col()
            .gap_2()
            .when(
                matches!(
                    self.object_tab,
                    ObjectTab::Relations | ObjectTab::IncomingRelations
                ),
                |body| {
                    body.child(
                        div()
                            .flex()
                            .gap_2()
                            .child(self.control(
                                "relations-incoming",
                                "Incoming",
                                Control::IncomingRelations,
                                false,
                                cx,
                            ))
                            .child(self.control(
                                "relations-outgoing",
                                "Outgoing",
                                Control::OutgoingRelations,
                                false,
                                cx,
                            ))
                            .when(self.related_object().is_some(), |row| {
                                row.child(self.control(
                                    "open-related-object",
                                    if self.object_tab == ObjectTab::IncomingRelations {
                                        "Open source table"
                                    } else {
                                        "Open referenced table"
                                    },
                                    Control::OpenRelated,
                                    false,
                                    cx,
                                ))
                            }),
                    )
                },
            )
            .child(self.metadata_table(cx))
            .into_any_element()
    }

    fn metadata_table(&self, cx: &Context<'_, Self>) -> AnyElement {
        let theme = Theme::of(cx);
        if !self
            .capabilities
            .contains(required_capability(self.object_tab))
        {
            return div()
                .p_3()
                .text_color(theme.colors.text_muted)
                .child(if self.object_tab == ObjectTab::Indexes {
                    "This session does not support index introspection."
                } else if self.object_tab == ObjectTab::Constraints {
                    "This session does not support constraint introspection."
                } else if self.object_tab == ObjectTab::IncomingRelations {
                    "This session does not support incoming foreign key discovery."
                } else {
                    "This session does not support foreign key introspection."
                })
                .into_any_element();
        }
        let count = self.selected_path.as_ref().and_then(|path| {
            let cache = self.catalog_cache.try_read()?;
            metadata_count(&cache, path, self.object_tab)
        });
        let headers = if self.object_tab == ObjectTab::Indexes {
            ["Index", "Columns", "Uniqueness", "Method"]
        } else if self.object_tab == ObjectTab::Constraints {
            ["Constraint", "Type", "Columns", "Status"]
        } else if self.object_tab == ObjectTab::IncomingRelations {
            ["From", "To", "Cardinality", ""]
        } else {
            [
                "Foreign key",
                "From columns",
                "Referenced object / columns",
                "On delete",
            ]
        };
        let mut body = div()
            .flex_1()
            .min_h_0()
            .flex()
            .flex_col()
            .gap_2()
            .child(self.structure_toolbar("refresh-object-metadata", cx));
        if self.catalog_active.is_some() {
            body = body.child(self.control(
                "cancel-object-metadata",
                "Cancel metadata loading",
                Control::CancelCatalog,
                false,
                cx,
            ));
        }
        if self.catalog_state == CatalogState::Cancelled {
            body = body.child(
                div()
                    .text_color(theme.colors.text_muted)
                    .child("Metadata loading cancelled. Previously loaded metadata is preserved."),
            );
        }
        if let CatalogState::Error(message) = &self.catalog_state {
            body = body.child(div().text_color(theme.colors.danger).child(message.clone()));
        }
        if self.object_tab == ObjectTab::IncomingRelations {
            body = body.child(div().text_size(theme.typography.small_size).text_color(theme.colors.text_muted).child("Incoming relationships declared by other tables. Cardinality uses direct unique keys with compatible comparisons; it is not a count or an integrity check. Enter opens the source table."));
        }
        if self.object_tab == ObjectTab::Relations {
            body = body.child(div().text_size(theme.typography.small_size).text_color(theme.colors.text_muted)
                .child("Outgoing foreign keys declared by this table. Incoming relationships are not included in this list."));
        }
        if self.object_tab == ObjectTab::Constraints {
            body = body.child(div().text_size(theme.typography.small_size).text_color(theme.colors.text_muted)
                .child("Definitions come from the database catalog or stored SQL. Unnamed means no declared name. CHECK column dependencies may be unreported. Select a row to inspect; ⌘C / Ctrl+C copies its definition."));
        }
        if let Some(definition) = self.selected_constraint_definition() {
            body = body
                .child(
                    div()
                        .flex()
                        .items_center()
                        .gap_2()
                        .child("Selected definition")
                        .child(
                            div()
                                .id("copy-constraint-definition")
                                .tab_index(0)
                                .cursor_pointer()
                                .px_2()
                                .border_1()
                                .border_color(theme.colors.border)
                                .focus(|style| style.border_color(theme.colors.border_focus))
                                .on_click(
                                    cx.listener(|this, _, _, cx| {
                                        this.copy_constraint_definition(cx)
                                    }),
                                )
                                .on_key_down(cx.listener(|this, event: &KeyDownEvent, _, cx| {
                                    if matches!(event.keystroke.key.as_str(), "enter" | "space") {
                                        this.copy_constraint_definition(cx);
                                        cx.stop_propagation();
                                    }
                                }))
                                .child("Copy definition"),
                        ),
                )
                .child(
                    div()
                        .id("constraint-definition")
                        .max_h(px(120.))
                        .overflow_y_scroll()
                        .child(definition),
                );
        }
        match count {
            Some(0) => body
                .child(if self.object_tab == ObjectTab::Indexes {
                    "No indexes reported for this object."
                } else if self.object_tab == ObjectTab::Constraints {
                    "No constraints reported for this object."
                } else if self.object_tab == ObjectTab::IncomingRelations {
                    "No incoming foreign keys reported for this object."
                } else {
                    "No outgoing foreign keys reported for this object."
                })
                .into_any_element(),
            None => body
                .child(if self.catalog_active.is_some() {
                    "Loading metadata…"
                } else {
                    "Metadata has not been loaded. Use Refresh structure to request it."
                })
                .into_any_element(),
            Some(count) => body
                .child(
                    div()
                        .id("object-metadata")
                        .text_size(px(13.))
                        .track_focus(&self.metadata_focus)
                        .tab_index(0)
                        .on_key_down(cx.listener(Self::on_metadata_key))
                        .focus(|style| style.border_color(theme.colors.border_focus))
                        .flex_1()
                        .min_h_0()
                        .flex()
                        .flex_col()
                        .border_1()
                        .border_color(theme.colors.border)
                        .overflow_hidden()
                        .child(
                            div()
                                .h(px(32.))
                                .flex_none()
                                .flex()
                                .items_center()
                                .bg(theme.colors.surface)
                                .children(
                                    headers
                                        .into_iter()
                                        .take(if self.object_tab == ObjectTab::IncomingRelations {
                                            3
                                        } else {
                                            4
                                        })
                                        .enumerate()
                                        .map(|(column, header)| {
                                            metadata_cell(self.object_tab, column)
                                                .text_color(theme.colors.text_muted)
                                                .child(header)
                                        }),
                                ),
                        )
                        .child(
                            uniform_list(
                                "object-metadata-rows",
                                count,
                                cx.processor(
                                    |this: &mut Self, range: std::ops::Range<usize>, _, cx| {
                                        let theme = Theme::of(cx);
                                        let Some(cache) = this.catalog_cache.try_read() else {
                                            return Vec::new();
                                        };
                                        let Some(path) = this.selected_path.as_ref() else {
                                            return Vec::new();
                                        };
                                        range
                                            .filter_map(|index| {
                                                metadata_row(&cache, path, this.object_tab, index)
                                                    .map(|row| (index, row))
                                            })
                                            .map(|(index, values)| {
                                                div()
                                                    .id(("metadata-row", index))
                                                    .on_click(cx.listener(
                                                        move |this, _, window, cx| {
                                                            this.metadata_selected = index;
                                                            window.focus(&this.metadata_focus);
                                                            cx.notify();
                                                        },
                                                    ))
                                                    .h(px(32.))
                                                    .w_full()
                                                    .flex()
                                                    .items_center()
                                                    .border_b_1()
                                                    .border_color(theme.colors.border)
                                                    .when(index == this.metadata_selected, |el| {
                                                        el.bg(theme.colors.selection)
                                                    })
                                                    .children(
                                                        values
                                                            .into_iter()
                                                            .take(
                                                                if this.object_tab
                                                                    == ObjectTab::IncomingRelations
                                                                {
                                                                    3
                                                                } else {
                                                                    4
                                                                },
                                                            )
                                                            .enumerate()
                                                            .map(|(column, value)| {
                                                                metadata_cell(
                                                                    this.object_tab,
                                                                    column,
                                                                )
                                                                .child(value)
                                                            }),
                                                    )
                                            })
                                            .collect::<Vec<_>>()
                                    },
                                ),
                            )
                            .track_scroll(self.metadata_scroll.clone())
                            .flex_1(),
                        ),
                )
                .into_any_element(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use oxyn_catalog::{Index, Relation, RelationKind};

    #[test]
    fn constraint_status_never_conflates_unknown_with_validated() {
        use oxyn_catalog::{Constraint, ConstraintKind};
        let path = CatalogPath::for_relation(None, Some("main"), "items").expect("path");
        let mut cache = CatalogCache::new();
        cache
            .set_relation(&path, Relation::new("items", RelationKind::Table))
            .expect("relation");
        for (validated, expected) in [
            (None, "Not reported"),
            (Some(false), "Not validated"),
            (Some(true), "Validated"),
        ] {
            let mut constraint =
                Constraint::new("present", ConstraintKind::PrimaryKey, vec!["id".into()]);
            constraint.validated = validated;
            cache
                .set_constraints(&path, vec![constraint])
                .expect("cache");
            let row = metadata_row(&cache, &path, ObjectTab::Constraints, 0).expect("row");
            assert_eq!(row.get(1).map(String::as_str), Some("PRIMARY KEY"));
            assert_eq!(row.last().map(String::as_str), Some(expected));
        }
    }

    #[test]
    fn metadata_distinguishes_unread_empty_and_populated() {
        let path = CatalogPath::for_relation(None, Some("main"), "items").expect("path");
        let mut cache = CatalogCache::new();
        cache
            .set_relation(&path, Relation::new("items", RelationKind::Table))
            .expect("relation");
        assert_eq!(metadata_count(&cache, &path, ObjectTab::Indexes), None);
        cache.set_indexes(&path, Vec::new()).expect("empty indexes");
        assert_eq!(metadata_count(&cache, &path, ObjectTab::Indexes), Some(0));
        cache
            .set_indexes(&path, vec![Index::new("odd\"; index", vec!["id".into()])])
            .expect("index");
        assert_eq!(metadata_count(&cache, &path, ObjectTab::Indexes), Some(1));
        assert_eq!(
            metadata_row(&cache, &path, ObjectTab::Indexes, 0)
                .expect("row")
                .first()
                .map(String::as_str),
            Some("odd\"; index")
        );
        assert!(metadata_row(&cache, &path, ObjectTab::Indexes, usize::MAX).is_none());
        assert_eq!(metadata_count(&cache, &path, ObjectTab::Relations), None);
    }
}
