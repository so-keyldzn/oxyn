//! Workspace content rendering using real session state.

use super::layout::Control;
use super::*;
use gpui::{AnyElement, FontWeight, div, px};
use oxyn_ui::{NEVER_EMULATED, Theme, surfaces};

/// Prefixes a title with the agent mark, when an agent wrote the text.
///
/// # Pourquoi la marque est un **préfixe**
///
/// [ADR-0023](../../../docs/adr/0023-fournisseurs-declares-et-provenance.md) et
/// [UX-SPEC](../../../docs/UX-SPEC.md#ce-quun-agent-a-écrit-reste-marqué) posent
/// qu'un texte écrit par un agent porte sa provenance, et qu'elle est visible
/// sur l'onglet. Elle était **écrite en base et montrée nulle part** : la
/// garantie annoncée était invérifiable par celui qu'elle protège.
///
/// Elle précède le titre parce qu'un onglet se tronque par la fin : un suffixe
/// serait le premier à disparaître, et disparaîtrait d'autant plus vite que le
/// titre est long. Les états, eux, restent en suffixe — ils sont transitoires,
/// la marque ne l'est pas.
///
/// Partagée avec la bibliothèque plutôt que recopiée : la marque doit se lire
/// **pareil** aux deux endroits qu'UX-SPEC nomme — l'onglet et la bibliothèque
/// —, et deux formulations finiraient par diverger sans que rien n'échoue.
pub(super) fn agent_marked(title: &str, from_agent: bool) -> String {
    if from_agent {
        format!("AI · {title}")
    } else {
        title.to_owned()
    }
}

/// Le libellé d'un onglet de console : son titre, sa marque, son état.
fn tab_label(title: &str, from_agent: bool, state: Option<&str>) -> String {
    let mut label = agent_marked(title, from_agent);
    if let Some(state) = state {
        label.push_str(" · ");
        label.push_str(state);
    }
    label
}

impl Workspace {
    pub(super) fn document_tabs(&self, cx: &Context<'_, Self>) -> AnyElement {
        let theme = Theme::of(cx);
        div()
            .id("document-tabs")
            .overflow_x_scroll()
            .track_scroll(&self.console_tabs_scroll)
            .h(px(40.))
            .flex_none()
            .flex()
            .items_center()
            .gap_1()
            .px_3()
            .bg(theme.colors.surface)
            .when_some(self.selected_path.as_ref(), |el, path| {
                el.child(
                    self.control(
                        "document-object",
                        path.relation()
                            .or(path.namespace())
                            .or(path.catalog())
                            .unwrap_or("Catalog")
                            .to_owned(),
                        Control::Object,
                        false,
                        cx,
                    ),
                )
            })
            .when(self.capabilities.contains(Capabilities::SQL), |el| {
                el.children(self.consoles.iter().map(|console| {
                    let selected = self.console == *console && self.panel == WorkspacePanel::Sql;
                    let view = console.read(cx);
                    let title = if view.title.trim().is_empty() {
                        "Untitled query"
                    } else {
                        &view.title
                    };
                    let label = tab_label(
                        title,
                        view.provenance.is_some(),
                        if view.document_closing {
                            Some("closing")
                        } else if view.save_active.is_some() {
                            Some("saving")
                        } else if view.awaiting_approval {
                            Some("approval")
                        } else if view.active.is_some() {
                            Some("running")
                        } else if view.dirty {
                            Some("unsaved")
                        } else {
                            None
                        },
                    );
                    let click_target = console.clone();
                    let key_target = console.clone();
                    let close_target = console.clone();
                    div()
                        .id(gpui::SharedString::from(format!("console-tab-{}", view.id)))
                        .tab_index(0)
                        .h(px(32.))
                        .min_w(px(164.))
                        .max_w(px(300.))
                        .rounded(px(6.))
                        .flex_none()
                        .px_3()
                        .flex()
                        .items_center()
                        .border_1()
                        .border_color(if selected {
                            theme.colors.border
                        } else {
                            theme.colors.surface
                        })
                        .bg(if selected {
                            theme.colors.background
                        } else {
                            theme.colors.surface
                        })
                        .text_color(if selected {
                            theme.colors.text
                        } else {
                            theme.colors.text_muted
                        })
                        .font_weight(FontWeight::MEDIUM)
                        .focus(|el| el.border_color(theme.colors.border_focus))
                        .cursor_pointer()
                        .on_click(cx.listener(move |this, _, window, cx| {
                            this.select_console(click_target.clone(), window, cx)
                        }))
                        .on_key_down(cx.listener(
                            move |this, event: &gpui::KeyDownEvent, window, cx| {
                                if matches!(event.keystroke.key.as_str(), "enter" | "space") {
                                    this.select_console(key_target.clone(), window, cx);
                                    cx.stop_propagation();
                                }
                            },
                        ))
                        .child(div().min_w_0().truncate().child(label))
                        .child(
                            div()
                                .id(("close-console", console.entity_id()))
                                // Atteignable au clavier, et actionnable : sans
                                // ces deux lignes, ce bouton était visible,
                                // cliquable et inutilisable autrement qu'à la
                                // souris — le point bloquant de
                                // `.claude/checklists/revue-ui.md`.
                                //
                                // `⌘W` ne le remplace pas : il ferme la console
                                // **active**, et seulement dans le panneau SQL.
                                // Fermer un autre onglet imposait donc de l'aller
                                // sélectionner d'abord, ou de prendre la souris.
                                //
                                // Sans test automatique, et c'est assumé : le
                                // vérifier demanderait d'atteindre ce bouton par
                                // tabulations depuis un point connu, or leur
                                // nombre dépend des onglets ouverts et des
                                // contrôles qui précèdent. Un test qui pose le
                                // focus par un clic, ou qui appelle
                                // `request_close_console` en direct, serait vert
                                // sans rien prouver de l'atteignabilité — le
                                // piège que `.claude/rules/tests.md` nomme. Cela
                                // relève de la recette clavier.
                                .tab_index(0)
                                .focus(|el| el.border_color(theme.colors.border_focus))
                                .ml_2()
                                .px_1()
                                .cursor_pointer()
                                .text_size(theme.typography.small_size)
                                .child("Close")
                                .on_key_down(cx.listener({
                                    let key_close = close_target.clone();
                                    move |this, event: &gpui::KeyDownEvent, window, cx| {
                                        if matches!(event.keystroke.key.as_str(), "enter" | "space")
                                        {
                                            this.request_close_console(
                                                key_close.clone(),
                                                window,
                                                cx,
                                            );
                                            cx.stop_propagation();
                                        }
                                    }
                                }))
                                .on_click(cx.listener(move |this, _, window, cx| {
                                    this.request_close_console(close_target.clone(), window, cx);
                                    cx.stop_propagation();
                                })),
                        )
                }))
                .child(if self.console_attempt.is_some() {
                    self.control(
                        "cancel-new-console",
                        "Cancel new console",
                        Control::CancelNewConsole,
                        false,
                        cx,
                    )
                } else {
                    self.control("new-console", "New console", Control::NewConsole, false, cx)
                })
            })
            .into_any_element()
    }

    /// The toolbar label of the bound-value editor, with what the next run sends.
    pub(super) fn parameters_label(&self, cx: &gpui::App) -> String {
        format!("Parameters · {}", self.console.read(cx).parameter_count(cx))
    }

    /// The bound-value editor of the active console, when it is open.
    ///
    /// It sits below the toolbar rather than over the editor: the values must
    /// stay readable next to the statement that uses them.
    fn parameters_panel(&self, cx: &Context<'_, Self>) -> AnyElement {
        let theme = Theme::of(cx);
        div()
            .id("query-parameters-panel")
            .max_h(px(300.))
            .overflow_y_scroll()
            .flex_none()
            .flex()
            .flex_col()
            .gap_2()
            .p_3()
            .rounded(px(8.))
            .border_1()
            .border_color(theme.colors.border)
            .child(
                div()
                    .text_size(theme.typography.small_size)
                    .text_color(theme.colors.text_muted)
                    .child(
                        "Values are bound by the driver. They are never inserted into the SQL \
                         text, saved with the query, or written to the history.",
                    ),
            )
            .child(self.console.read(cx).parameters.clone())
            .into_any_element()
    }

    pub(super) fn body(&self, cx: &Context<'_, Self>) -> AnyElement {
        let theme = Theme::of(cx);
        match self.panel {
            WorkspacePanel::Sql if self.consoles.is_empty() => div().flex_1().flex().flex_col().items_center().justify_center().gap_3()
                .child("Open a console to write a query.")
                .child(self.control("empty-new-console", "New console · ⌘T", Control::NewConsole, false, cx)).into_any_element(),
            WorkspacePanel::Sql if self.capabilities.contains(Capabilities::SQL) => div()
                .flex_1()
                .min_h_0()
                .flex()
                .flex_col()
                .gap_2()
                .p_3()
                .child(
                    div()
                        .flex()
                        .items_center()
                        .gap_2()
                        .flex_none()
                        // Figma `273:37036`: Run, Stop and Explain form one
                        // 286 px group of three 96 px segments. Stop is its own
                        // control rather than a Run that changes meaning: a
                        // button whose label swaps under the pointer is one
                        // mis-click away from starting what you meant to stop.
                        .child(self.control(
                            "run-query",
                            "Run · ⌘Enter",
                            Control::Run,
                            false,
                            cx,
                        ))
                        .child(self.control("stop-query", "Stop", Control::Stop, false, cx))
                        .child(self.control(
                            "explain-query",
                            "Explain query",
                            Control::Explain,
                            false,
                            cx,
                        ))
                        .child(self.control(
                            "query-parameters",
                            self.parameters_label(cx),
                            Control::Parameters,
                            false,
                            cx,
                        ))
                        // Le relevé `191:1521` place cette mention après
                        // `Parameters`. Elle dit une chose que rien d'autre ne
                        // dit dans la console : la **connexion** est en lecture
                        // seule, donc `Run` refusera une écriture avant même de
                        // l'envoyer. Sans elle, l'utilisateur écrit son `UPDATE`
                        // et découvre le refus à l'exécution.
                        .when(self.read_only, |el| {
                            el.child(
                                div()
                                    .flex_none()
                                    .text_size(theme.typography.small_size)
                                    .text_color(theme.colors.text_muted)
                                    .child("Read-only console"),
                            )
                        })
                        .child(self.control(
                            "query-columns",
                            self.columns_label(ResultSource::Query, cx),
                            Control::Columns,
                            false,
                            cx,
                        ))
                        .child(self.control(
                            "query-inspector",
                            "Inspect row",
                            Control::Inspector,
                            false,
                            cx,
                        ))
                        .child(self.control(
                            "toggle-format-settings",
                            if self.settings_open {
                                "Hide display settings"
                            } else {
                                "Display settings"
                            },
                            Control::FormatSettings,
                            false,
                            cx,
                        ))
                        // Figma `191:2003`: at the right end of the 1272 px bar.
                        // Absent, never greyed, when the session cannot declare
                        // where it resolves names (ADR-0003, ADR-0019).
                        .children(self.session_context_control(cx)),
                )
                .children(self.session_context_notice(cx))
                .child(div().flex_none().flex().flex_wrap().items_center().gap_2()
                    .child("Query name")
                    .child(div().w(px(240.)).child(self.console.read(cx).name.clone()))
                    .child(self.control("save-query", if self.console.read(cx).document_closing { "Cancel closing" } else if self.console.read(cx).save_active.is_some() { "Cancel save" } else if self.console.read(cx).save_conflict { "Save as new query" } else { "Save query · ⌘S" },
                        if self.console.read(cx).document_closing { Control::CancelDocumentClose } else if self.console.read(cx).save_active.is_some() { Control::CancelSave } else if self.console.read(cx).save_conflict { Control::SaveConsoleCopy } else { Control::SaveConsole }, false, cx))
                    .child(div().text_size(theme.typography.small_size).text_color(theme.colors.text_muted).child(self.console.read(cx).save_notice.clone())))
                .child(div().flex_none().text_size(theme.typography.small_size).text_color(theme.colors.text_muted).child(self.console.read(cx).draft_notice.clone()))
                .when(self.console.read(cx).parameters_open, |el| {
                    el.child(self.parameters_panel(cx))
                })
                .when(self.columns_open, |el| {
                    el.child(self.column_manager(ResultSource::Query, cx))
                })
                .when(self.settings_open, |element| {
                    element.child(
                        div()
                            .id("query-display-settings")
                            .max_h(px(300.))
                            .overflow_y_scroll()
                            .flex_none()
                            .flex()
                            .flex_col()
                            .gap_3()
                            .child(self.reading_settings(cx))
                            .child(self.settings.clone()),
                    )
                })
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
                        .text_size(theme.typography.small_size)
                        .flex()
                        .items_center()
                        .justify_between()
                        .child("RESULTS")
                        .child(self.control(
                            "query-text-size",
                            self.reading_label(cx),
                            Control::ToggleReading,
                            false,
                            cx,
                        )),
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
                        .child(self.result_area(ResultSource::Query, cx))
                        .child(self.export.clone()),
                )
                .into_any_element(),
            WorkspacePanel::Assistant => self.assistant_panel(cx),
            WorkspacePanel::Object => self.object_details(cx),
            WorkspacePanel::Preferences => self.preferences_panel(cx),
            WorkspacePanel::Library => div().flex_1().min_h_0().flex().child(self.library.clone()).into_any_element(),
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
                        "⌘T — Open a console with its own session",
                        "Ctrl+Tab / Ctrl+Shift+Tab — Switch consoles",
                        "⌘W — Close the current console; unsaved SQL requires a choice",
                        "⌘⇧L — Switch light / dark appearance",
                        "⌘Enter — Run the current statement",
                        "Escape — Cancel a running query from the editor",
                        "Tab / Shift+Tab — Move between controls; Tab in the editor indents",
                        "Catalog: select a table to preview its first 200 rows",
                        "⌘2 — Focus the table preview; Escape cancels its loading",
                        "⌘⇧H — Browse local history and saved queries without replacing the current console.",
                    ]
                    .map(|text| div().child(text)),
                )
                .child(self.session_support(cx))
                .into_any_element(),
            _ => div()
                .flex_1()
                .p_6()
                .child("SQL is not supported by this session.")
                .into_any_element(),
        }
    }

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
                    .text_size(theme.typography.small_size)
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

#[cfg(test)]
mod tests {
    use super::{super::preview::preview_notice, tab_label};

    /// Le pied d'un aperçu dit que le total n'a **pas** été demandé.
    ///
    /// Relevé `190:1163`. Sans cette mention, un aperçu de 200 lignes sur une
    /// table qui en contient cinquante millions ressemble en tout point à un
    /// aperçu de 200 lignes sur une table qui en contient 200 — et Oxyn ne
    /// compte jamais les lignes pour l'afficher, parce qu'un `COUNT(*)` sur
    /// cinquante millions de lignes est une requête que personne n'a demandée.
    #[test]
    fn le_pied_d_un_apercu_ne_laisse_pas_croire_a_un_total() {
        let pied = preview_notice(200, std::time::Duration::from_millis(284));
        assert!(pied.contains("200 rows shown"), "{pied}");
        assert!(
            pied.contains("Total count not requested"),
            "le nombre affiché n'est pas celui de la table, et il faut le dire : {pied}"
        );
        // La durée vient de la maquette (`190:1860`) et manquait : elle
        // distingue une base lente d'un aperçu qui n'a rien trouvé.
        assert!(pied.contains("284 ms"), "{pied}");
        // Et elle ne redit pas la lecture seule : la barre Data la porte déjà,
        // quarante pixels plus haut. La maquette ne l'écrit qu'une fois.
        assert!(
            !pied.contains("Read only"),
            "mention dupliquée avec la barre Data : {pied}"
        );
    }

    /// La marque d'un agent se voit, et elle survit à un onglet tronqué.
    ///
    /// Trouvé par la relecture des divergences : la provenance était écrite en
    /// base, protégée par un `coalesce`, testée dans le store — et **montrée
    /// nulle part**. ADR-0023 existe pour qu'on sache six mois plus tard qu'un
    /// agent a écrit ce `SELECT` ; la donnée était là, illisible depuis Oxyn.
    #[test]
    fn un_onglet_dit_quand_un_agent_a_ecrit_son_texte() {
        assert_eq!(tab_label("rapport.sql", false, None), "rapport.sql");
        assert_eq!(tab_label("rapport.sql", true, None), "AI · rapport.sql");

        // Les états restent en suffixe, la marque en préfixe : un onglet se
        // tronque par la fin, et c'est l'état qui doit céder en premier — il
        // est transitoire, la provenance ne l'est pas.
        assert_eq!(
            tab_label("rapport.sql", true, Some("running")),
            "AI · rapport.sql · running"
        );
        assert_eq!(
            tab_label("rapport.sql", false, Some("unsaved")),
            "rapport.sql · unsaved"
        );
    }
}
