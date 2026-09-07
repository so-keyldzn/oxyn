//! Workspace content rendering using real session state.

use super::layout::Control;
use super::*;
use gpui::{AnyElement, FontWeight, div, px};
use oxyn_ui::{NEVER_EMULATED, Theme, surfaces};

impl Workspace {
    pub(super) fn body(&self, cx: &Context<'_, Self>) -> AnyElement {
        let theme = Theme::of(cx);
        match self.panel {
            WorkspacePanel::Sql if self.capabilities.contains(Capabilities::SQL) => div()
                .flex_1()
                .min_h_0()
                .flex()
                .flex_col()
                .gap_4()
                .p_6()
                .child(
                    div()
                        .flex()
                        .items_center()
                        .justify_between()
                        .flex_none()
                        .child(
                            div()
                                .flex()
                                .flex_col()
                                .gap_1()
                                .child(
                                    div()
                                        .text_size(px(24.))
                                        .font_weight(FontWeight::SEMIBOLD)
                                        .child("SQL editor"),
                                )
                                .child(div().text_color(theme.colors.text_muted).child(format!(
                                    "{} · {}",
                                    self.display.driver,
                                    if self.read_only {
                                        "Read-only session"
                                    } else {
                                        "Current session"
                                    }
                                ))),
                        )
                        .child(self.control(
                            "run-query",
                            if self.active.is_some() {
                                "Cancel execution"
                            } else {
                                "Run query · ⌘Enter"
                            },
                            Control::Run,
                            false,
                            cx,
                        )),
                )
                .child(
                    div()
                        .h(px(220.))
                        .min_h(px(80.))
                        .flex_shrink()
                        .border_1()
                        .border_color(theme.colors.border)
                        .rounded(px(8.))
                        .overflow_hidden()
                        .child(self.editor.clone()),
                )
                .child(
                    div()
                        .flex_none()
                        .text_color(theme.colors.text_muted)
                        .text_size(px(11.))
                        .child("RESULTS"),
                )
                .child(
                    div()
                        .flex_1()
                        .min_h_0()
                        .flex()
                        .flex_col()
                        .border_1()
                        .border_color(theme.colors.border)
                        .rounded(px(8.))
                        .overflow_hidden()
                        .child(div().flex_1().min_h_0().child(self.grid.clone()))
                        // Sous la grille et non dans une barre d'outils : le
                        // geste porte sur le résultat qu'on voit, et ce qu'il
                        // peut exporter dépend de l'état de ce résultat.
                        .child(self.export.clone()),
                )
                .into_any_element(),
            WorkspacePanel::Object => self.object_details(cx),
            WorkspacePanel::Help => div()
                .flex_1()
                .p_6()
                .flex()
                .flex_col()
                .gap_4()
                .child(
                    div()
                        .text_size(px(24.))
                        .font_weight(FontWeight::SEMIBOLD)
                        .child("Workspace guide"),
                )
                .children(
                    [
                        "⌘B — Expand or collapse the sidebar",
                        "⌘J — Focus the SQL editor",
                        "⌘1 — Focus the catalog",
                        "⌘⇧L — Switch light / dark appearance",
                        "⌘Enter — Run the current statement",
                        "Escape — Cancel a running query from the editor",
                        "Tab / Shift+Tab — Move between controls; Tab in the editor indents",
                        "Catalog: arrows navigate and expand; Enter opens object details",
                        "Export: choose a format under the results, once a result is complete",
                        "History and saved queries are not available in this version.",
                    ]
                    .map(|text| div().child(text)),
                )
                .child(self.session_support(cx))
                .into_any_element(),
            // Une source qui ne parle pas SQL n'a pas d'éditeur, mais elle a
            // des capacités, et les taire laisserait l'écran vide sans dire
            // pourquoi.
            _ => div()
                .flex_1()
                .p_6()
                .flex()
                .flex_col()
                .gap_4()
                .child("SQL is not supported by this session.")
                .child(self.session_support(cx))
                .into_any_element(),
        }
    }

    fn object_details(&self, cx: &Context<'_, Self>) -> AnyElement {
        let theme = Theme::of(cx);
        let path = self.selected_path.as_ref();
        let title = path
            .and_then(|p| p.relation().or(p.namespace()).or(p.catalog()))
            .unwrap_or("Catalog");
        let metadata = path.and_then(|path| {
            let cache = self.catalog_cache.try_read()?;
            let summary = cache.relation_summary(path)?;
            let mut detail = format!("{} · {}", summary.kind.as_str(), self.display.driver);
            if let Some(relation) = cache.relation(path) {
                detail.push_str(&format!(" · {} columns", relation.fields.len()));
                if let Some(rows) = relation
                    .estimated_rows
                    .filter(|_| self.capabilities.contains(Capabilities::ROW_COUNT_ESTIMATE))
                {
                    detail.push_str(&format!(" · approximately {rows} rows"));
                }
            }
            Some(detail)
        });
        div().flex_1().p_6().flex().flex_col().gap_4()
            .child(div().text_size(px(24.)).font_weight(FontWeight::SEMIBOLD).child(title.to_owned()))
            .child(div().text_color(theme.colors.text_muted).child(path.map(|p| p.to_string()).unwrap_or_default()))
            .when_some(metadata, |el, detail| el.child(div().text_color(theme.colors.text_muted).child(detail)))
            .child(div().border_1().border_color(theme.colors.border).rounded(px(8.)).p_4().flex().flex_col().gap_2()
                .child("Selected catalog object")
                .child(div().text_color(theme.colors.text_muted).child("Data preview is not available yet. Use the SQL editor to write and run a query."))
                .child(div().text_size(px(11.)).text_color(theme.colors.text_muted).child("Selecting an object does not execute SQL.")))
            .when_some(path.filter(|p| p.relation().is_some()), |el, path| el.child(self.structure(path, cx)))
            .when(path.is_some_and(|p| p.relation().is_some()) && self.catalog_active.is_none(), |el| el.child(self.control("object-structure", "Load structure details", Control::Describe, false, cx)))
            .when(self.capabilities.contains(Capabilities::SQL), |el| el.child(self.control("object-sql", "Open SQL editor · ⌘J", Control::Sql, false, cx)))
            .into_any_element()
    }

    /// Indexes, constraints and foreign keys of the selected relation.
    ///
    /// Each is governed by its own capability, and each states which of the five
    /// states it is in ([UX-SPEC](../../../docs/UX-SPEC.md#états-dune-vue)). The
    /// distinction that matters here is between *never read* and *read and
    /// empty*: the cache returns `None` for the first and an empty slice for the
    /// second, and saying "no index" about a source that was never asked would
    /// be the lie the capability model exists to prevent.
    fn structure(&self, path: &CatalogPath, cx: &Context<'_, Self>) -> AnyElement {
        let theme = Theme::of(cx);
        let loading = self.catalog_active.is_some();
        // `try_read` and not `read`: the UI thread never waits on a lock
        // ([I-05](../../../CLAUDE.md#i-05)). A frame that misses the lock says
        // so instead of borrowing the "not read yet" sentence — the writer is
        // publishing, and the next frame will have the answer.
        let cache = self.catalog_cache.try_read();
        let occupe = cache.is_none();
        let mut lignes: Vec<(&'static str, String)> = Vec::new();

        for (nom, capability, lu) in [
            (
                "Indexes",
                Capabilities::INDEXES,
                cache.as_ref().and_then(|cache| {
                    cache.indexes(path).map(|index| {
                        index
                            .iter()
                            .map(|index| {
                                let unique = if index.unique { "unique " } else { "" };
                                format!("{} ({}{})", index.name, unique, index.fields.join(", "))
                            })
                            .collect::<Vec<_>>()
                    })
                }),
            ),
            (
                "Foreign keys",
                Capabilities::FOREIGN_KEYS,
                cache.as_ref().and_then(|cache| {
                    cache.foreign_keys(path).map(|cles| {
                        cles.iter()
                            .map(|cle| format!("{} ({})", cle.name, cle.fields.join(", ")))
                            .collect::<Vec<_>>()
                    })
                }),
            ),
            // No `list_constraints` on `CatalogProvider` yet, so the surface can
            // only say what the session declares — never a count it does not
            // have.
            ("Constraints", Capabilities::CONSTRAINTS, None),
        ] {
            let etat = if !self.capabilities.contains(capability) {
                format!("Unsupported — not introspectable on this session. {NEVER_EMULATED}")
            } else {
                match lu {
                    Some(valeurs) if valeurs.is_empty() => {
                        "None — the server reported none.".to_owned()
                    }
                    Some(valeurs) => valeurs.join(" · "),
                    None if loading => "Loading…".to_owned(),
                    None if occupe => "Refreshing…".to_owned(),
                    // L'erreur du dernier rafraîchissement, montrée ici et pas
                    // seulement dans l'arbre : sans elle, une lecture qui a
                    // échoué se lit « pas encore demandée », et l'utilisateur
                    // redemande sans savoir que le serveur a déjà refusé.
                    None => match &self.catalog_state {
                        CatalogState::Error(message) => {
                            format!("{message}\nLoad the structure details to retry.")
                        }
                        _ => "Not read yet — load the structure details.".to_owned(),
                    },
                }
            };
            lignes.push((nom, etat));
        }

        div()
            .border_1()
            .border_color(theme.colors.border)
            .rounded(px(8.))
            .p_4()
            .flex()
            .flex_col()
            .gap_2()
            .child(div().font_weight(FontWeight::SEMIBOLD).child("Structure"))
            .children(lignes.into_iter().map(|(nom, etat)| {
                div()
                    .flex()
                    .flex_row()
                    .gap_2()
                    .child(div().w(px(120.)).flex_none().child(nom))
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .text_color(theme.colors.text_muted)
                            .child(etat),
                    )
            }))
            .into_any_element()
    }

    /// What this session declares, gaps included.
    ///
    /// Listed rather than left to be discovered on the statement that fails:
    /// what a source cannot do is part of what a professional needs to know
    /// before writing, not after ([ADR-0003](../../../docs/adr/0003-driver-capabilities.md)).
    fn session_support(&self, cx: &Context<'_, Self>) -> AnyElement {
        let theme = Theme::of(cx);
        div()
            .border_1()
            .border_color(theme.colors.border)
            .rounded(px(8.))
            .p_4()
            .flex()
            .flex_col()
            .gap_2()
            .child(
                div()
                    .font_weight(FontWeight::SEMIBOLD)
                    .child("Session capabilities"),
            )
            .child(
                div()
                    .text_size(px(11.))
                    .text_color(theme.colors.text_muted)
                    .child(NEVER_EMULATED),
            )
            .children(surfaces(self.capabilities).into_iter().map(|surface| {
                div()
                    .flex()
                    .flex_row()
                    .gap_2()
                    .child(div().w(px(160.)).flex_none().child(surface.surface))
                    .child(
                        div()
                            .w(px(96.))
                            .flex_none()
                            // Le mot, pas seulement la couleur : une information
                            // portée par la seule couleur n'existe pas pour tout
                            // le monde (revue-ui, accessibilité).
                            .text_color(if surface.supported {
                                theme.colors.success
                            } else {
                                theme.colors.warning
                            })
                            .child(if surface.supported {
                                "Supported"
                            } else {
                                "Unsupported"
                            }),
                    )
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .text_color(theme.colors.text_muted)
                            .child(surface.detail),
                    )
            }))
            .into_any_element()
    }
}
