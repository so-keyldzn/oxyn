//! Capability-aware metadata tabs, reading only visible rows from the shared cache.

use super::layout::Control;
use super::preview::PREVIEW_ROWS;
use super::*;
use gpui::{AnyElement, ClipboardItem, KeyDownEvent, ScrollStrategy, div, px, uniform_list};
use oxyn_catalog::CatalogCache;
use oxyn_catalog::path::{QuoteStyle, quote_identifier};
use oxyn_core::SqlDialect;
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

/// Les deux synthèses que la maquette place sous la liste des contraintes.
///
/// Relevé `229:7637` : « NOT NULL columns » suivi des noms, et « Unique indexes »
/// suivi d'une phrase qui renvoie vers l'onglet Indexes. Elles ne demandent
/// **aucune lecture** — tout est déjà dans le cache, écrit par l'introspection
/// qui a rempli la table au-dessus.
///
/// Ce qu'elles apportent, et que la liste des contraintes ne dit pas : un
/// `NOT NULL` n'est pas une ligne de `pg_constraint` sur PostgreSQL — c'est un
/// attribut de colonne. Une table dont chaque colonne est obligatoire affiche
/// donc une liste de contraintes qui n'en mentionne aucune, et l'utilisateur en
/// conclut qu'il n'y en a pas. Un index unique, lui, est bien une contrainte,
/// mais il vit dans un autre onglet : le dire évite de le chercher ici.
///
/// Rend `None` quand la relation n'est pas encore lue — il n'y a alors rien à
/// résumer, et afficher « aucune colonne obligatoire » serait faux.
fn constraint_summaries(
    cache: &CatalogCache,
    path: &CatalogPath,
) -> Option<(Vec<String>, Vec<String>)> {
    let relation = cache.relation(path)?;
    let obligatoires = relation
        .fields
        .iter()
        .filter(|field| !field.nullable)
        .map(|field| field.name.clone())
        .collect();
    let uniques = cache
        .indexes(path)
        .unwrap_or_default()
        .iter()
        .filter(|index| index.unique)
        .map(|index| index.name.clone())
        .collect();
    Some((obligatoires, uniques))
}

/// La phrase qui décrit la relation sélectionnée.
///
/// Relevé `229:32690` : « public.orders → public.customers » suivi d'une phrase
/// sur la clé étrangère. La liste au-dessus donne les colonnes ; elle ne dit pas
/// **dans quel sens** la relation va, et c'est tout ce qui compte pour savoir
/// laquelle des deux tables porte la contrainte.
///
/// La flèche part toujours de la table qui **porte** la clé vers celle qui est
/// référencée, quel que soit l'onglet : en relations entrantes, la table
/// affichée est la cible, et inverser la flèche selon l'onglet donnerait deux
/// lectures contradictoires du même lien.
fn selected_relationship(
    cache: &CatalogCache,
    path: &CatalogPath,
    tab: ObjectTab,
    index: usize,
) -> Option<String> {
    let rendu = |source: &CatalogPath, cible: &CatalogPath, colonnes: &[String], action: &str| {
        format!(
            "{} → {} · {} · on delete {action}",
            source,
            cible,
            colonnes.join(", ")
        )
    };
    match tab {
        ObjectTab::Relations => {
            let cle = cache.foreign_keys(path)?.get(index)?;
            Some(rendu(
                path,
                &cle.references.relation,
                &cle.fields,
                cle.on_delete.as_str(),
            ))
        }
        ObjectTab::IncomingRelations => {
            let cle = cache.incoming_foreign_keys(path)?.get(index)?;
            Some(rendu(
                &cle.source,
                path,
                &cle.key.fields,
                cle.key.on_delete.as_str(),
            ))
        }
        _ => None,
    }
}

/// La requête bornée que `229:32690` montre sous la relation sélectionnée.
///
/// # Ce qu'elle est, et ce qu'elle n'est pas
///
/// Un **modèle à compléter**, jamais une requête à lancer. Elle nomme la table
/// qui porte la clé, ses colonnes, et laisse la valeur à saisir — parce
/// qu'Oxyn ne l'a pas : la maquette elle-même écrit `WHERE customer_i…` sans
/// valeur. Elle n'est exécutée par aucun chemin ; `Review related-row query`
/// l'ouvre dans une console, où l'utilisateur la lit, la complète et l'exécute
/// lui-même ([I-07](../../../CLAUDE.md#i-07)).
///
/// # Les identifiants sont cités
///
/// Ils viennent du catalogue, donc du serveur : les concaténer tels quels
/// violerait [I-10](../../../CLAUDE.md#i-10), y compris dans un texte qui ne
/// s'exécute pas — une table nommée `"users"; DROP TABLE audit; --` produirait
/// un modèle qui fait exactement cela au premier clic sur `Run`.
/// `quote_identifier` est la fonction que les deux drivers emploient déjà pour
/// composer leurs aperçus ; le style suit le dialecte de la session.
///
/// # La borne
///
/// `LIMIT 200`, la même que l'aperçu (`PREVIEW_ROWS`). « Bounded » est dans le
/// nom de la planche : ouvrir un modèle sans borne sur une table liée de
/// plusieurs millions de lignes offrirait en un clic le `SELECT *` que
/// [I-06](../../../CLAUDE.md#i-06) passe son temps à empêcher.
fn related_row_query(
    cache: &CatalogCache,
    path: &CatalogPath,
    tab: ObjectTab,
    index: usize,
    dialect: SqlDialect,
) -> Option<String> {
    let style = QuoteStyle::for_dialect(dialect);
    let cite = |chemin: &CatalogPath| {
        let mut morceaux = Vec::new();
        if let Some(namespace) = chemin.namespace() {
            morceaux.push(quote_identifier(namespace, style));
        }
        morceaux.push(quote_identifier(chemin.relation()?, style));
        Some(morceaux.join("."))
    };

    let (porteuse, colonnes) = match tab {
        ObjectTab::Relations => {
            let cle = cache.foreign_keys(path)?.get(index)?;
            (path.clone(), cle.fields.clone())
        }
        ObjectTab::IncomingRelations => {
            let cle = cache.incoming_foreign_keys(path)?.get(index)?;
            (cle.source.clone(), cle.key.fields.clone())
        }
        _ => return None,
    };
    if colonnes.is_empty() {
        return None;
    }

    // Une clé composite donne autant de conditions que de colonnes : en omettre
    // une rendrait le modèle silencieusement plus large que la relation.
    let conditions = colonnes
        .iter()
        .map(|colonne| format!("{} = ", quote_identifier(colonne, style)))
        .collect::<Vec<_>>()
        .join("\n  AND ");

    Some(format!(
        "SELECT *\nFROM {}\nWHERE {conditions}\nLIMIT {PREVIEW_ROWS};",
        cite(&porteuse)?
    ))
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
        // Before the early returns below: the sub-tab is part of the restored
        // location whether or not this session can load what it shows.
        self.persist_location(cx);
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
            .children(self.selected_relationship_line(cx))
            .children(self.constraint_summaries_panel(cx))
            .into_any_element()
    }

    /// La ligne « Selected relationship » de `229:32690`.
    fn selected_relationship_line(&self, cx: &Context<'_, Self>) -> Option<AnyElement> {
        if !matches!(
            self.object_tab,
            ObjectTab::Relations | ObjectTab::IncomingRelations
        ) {
            return None;
        }
        let theme = Theme::of(cx);
        let path = self.selected_path.as_ref()?;
        let phrase = {
            let cache = self.catalog_cache.try_read()?;
            selected_relationship(&cache, path, self.object_tab, self.metadata_selected)?
        };
        Some(
            div()
                .flex_none()
                .flex()
                .flex_col()
                .gap_1()
                .pt_2()
                .child(
                    div()
                        .font_weight(gpui::FontWeight::MEDIUM)
                        .child("Selected relationship"),
                )
                .child(
                    div()
                        .text_size(theme.typography.small_size)
                        .text_color(theme.colors.text_muted)
                        .child(phrase),
                )
                .children(self.related_row_preview(cx))
                .into_any_element(),
        )
    }

    /// Ouvre le modèle de requête liée dans une console. **Rien ne s'exécute.**
    pub(super) fn open_related_row_query(&mut self, cx: &mut Context<'_, Self>) {
        let Some(path) = self.selected_path.clone() else {
            return;
        };
        let Some(requete) = self.catalog_cache.try_read().and_then(|cache| {
            related_row_query(
                &cache,
                &path,
                self.object_tab,
                self.metadata_selected,
                self.dialect,
            )
        }) else {
            return;
        };
        self.open_library_query(
            library::OpenQuery::Copy {
                text: requete,
                title: "Related rows.sql".into(),
                origin: "a related-row template · complete the condition before running".into(),
                // Un modèle composé par Oxyn n'est écrit ni par l'utilisateur ni
                // par un agent : il n'a pas de provenance à porter.
                provenance: None,
            },
            cx,
        );
    }

    /// L'aperçu de requête liée de `229:32690`, et le bouton qui l'ouvre.
    ///
    /// Le modèle est **montré avant d'être ouvert** : c'est ce qui permet de
    /// juger sans rien déclencher. Le bouton l'envoie dans une console, où rien
    /// ne s'exécute tant que l'utilisateur ne le demande pas
    /// ([I-07](../../../CLAUDE.md#i-07)) — le même chemin que `Open DDL in
    /// console`.
    fn related_row_preview(&self, cx: &Context<'_, Self>) -> Option<AnyElement> {
        let theme = Theme::of(cx);
        let path = self.selected_path.as_ref()?;
        let requete = {
            let cache = self.catalog_cache.try_read()?;
            related_row_query(
                &cache,
                path,
                self.object_tab,
                self.metadata_selected,
                self.dialect,
            )?
        };
        Some(
            div()
                .flex_none()
                .flex()
                .flex_col()
                .gap_1()
                .pt_2()
                .child(
                    div()
                        .font_weight(gpui::FontWeight::MEDIUM)
                        .child("Bounded related-row preview"),
                )
                .child(
                    div()
                        .p_2()
                        .rounded(theme.radii.control)
                        .border_1()
                        .border_color(theme.colors.border)
                        .font_family(theme.typography.mono_family.clone())
                        .text_size(theme.typography.mono_size)
                        .text_color(theme.colors.text_muted)
                        .child(requete),
                )
                .child(div().flex().child(self.control(
                    "review-related-query",
                    "Review related-row query",
                    Control::ReviewRelatedQuery,
                    false,
                    cx,
                )))
                .into_any_element(),
        )
    }

    /// Les deux synthèses de `229:7637`, sous la liste des contraintes.
    fn constraint_summaries_panel(&self, cx: &Context<'_, Self>) -> Option<AnyElement> {
        if self.object_tab != ObjectTab::Constraints {
            return None;
        }
        let theme = Theme::of(cx);
        let path = self.selected_path.as_ref()?;
        let (obligatoires, uniques) = {
            // `try_read` et non `read` : le catalogue est partagé, et le fil
            // d'interface ne bloque jamais sur un verrou (I-05). Sans la
            // synthèse, la liste des contraintes reste lisible.
            let cache = self.catalog_cache.try_read()?;
            constraint_summaries(&cache, path)?
        };

        let titre = |texte: &'static str| div().font_weight(gpui::FontWeight::MEDIUM).child(texte);
        let detail = |texte: String| {
            div()
                .text_size(theme.typography.small_size)
                .text_color(theme.colors.text_muted)
                .child(texte)
        };

        Some(
            div()
                .flex_none()
                .flex()
                .flex_col()
                .gap_1()
                .pt_2()
                .child(titre("NOT NULL columns"))
                .child(detail(if obligatoires.is_empty() {
                    "None: every column accepts NULL.".to_owned()
                } else {
                    obligatoires.join(" · ")
                }))
                .child(titre("Unique indexes"))
                .child(detail(if uniques.is_empty() {
                    "None. Unique indexes, when there are any, are listed under Indexes.".to_owned()
                } else {
                    format!("{} · listed under Indexes.", uniques.join(" · "))
                }))
                // `Open Indexes` (`229:8021`) : la maquette offre le geste, le
                // code ne donnait que la phrase. Dire « c'est dans un autre
                // onglet » sans y mener oblige à revenir à la barre et à s'y
                // repérer, juste après avoir lu qu'on regardait au mauvais
                // endroit. Le contrôle est celui de la barre d'onglets — même
                // `Control::Indexes`, donc aucun second chemin.
                .when(self.capabilities.contains(Capabilities::INDEXES), |el| {
                    el.child(div().pt_1().child(self.control(
                        "constraints-open-indexes",
                        "Open Indexes",
                        Control::Indexes,
                        false,
                        cx,
                    )))
                })
                .into_any_element(),
        )
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

    /// Les deux synthèses disent ce que la liste des contraintes ne dit pas.
    ///
    /// Relevé `229:7637`. L'intérêt n'est pas décoratif : sur PostgreSQL, un
    /// `NOT NULL` n'est **pas** une ligne de `pg_constraint` — c'est un attribut
    /// de colonne. Une table dont chaque colonne est obligatoire affiche donc
    /// une liste de contraintes qui n'en mentionne aucune, et l'utilisateur en
    /// conclut qu'il n'y en a pas.
    #[test]
    fn les_synthese_de_contraintes_montrent_ce_que_la_liste_tait() {
        use oxyn_catalog::{Field, LogicalType};

        let path = CatalogPath::for_relation(None, Some("public"), "clients").expect("path");
        let mut cache = CatalogCache::new();
        cache
            .set_relation(
                &path,
                Relation::new("clients", RelationKind::Table).with_fields(vec![
                    Field::new("id", 0, LogicalType::INT64, "int8").primary_key(),
                    Field::new("email", 1, LogicalType::Text, "text"),
                    Field::new("note", 2, LogicalType::Text, "text"),
                ]),
            )
            .expect("relation");

        let mut unique = Index::new("clients_email_idx", vec!["email".into()]);
        unique.unique = true;
        let ordinaire = Index::new("clients_note_idx", vec!["note".into()]);
        cache
            .set_indexes(&path, vec![unique, ordinaire])
            .expect("indexes");

        let (obligatoires, uniques) =
            constraint_summaries(&cache, &path).expect("la relation est lue");

        // `primary_key()` implique non nul ; les deux autres colonnes sont
        // nullables par défaut — le parti prudent du modèle.
        assert_eq!(obligatoires, vec!["id".to_owned()]);
        // Seul l'index unique compte : l'autre n'est pas une contrainte.
        assert_eq!(uniques, vec!["clients_email_idx".to_owned()]);
    }

    /// Une relation non encore lue ne se résume pas.
    ///
    /// Le piège que ce test ferme : rendre `Some((vec![], vec![]))` afficherait
    /// « aucune colonne obligatoire » sur une table dont on ne sait encore rien
    /// — une affirmation fausse, indiscernable d'une table réellement toute
    /// nullable.
    #[test]
    fn une_relation_non_lue_ne_produit_aucune_synthese() {
        let path = CatalogPath::for_relation(None, Some("public"), "inconnue").expect("path");
        assert!(constraint_summaries(&CatalogCache::new(), &path).is_none());
    }

    /// La flèche part de qui **porte** la clé, dans les deux onglets.
    ///
    /// Relevé `229:32690`. Le piège qu'un test ferme ici : inverser la flèche
    /// selon l'onglet paraît naturel — on regarde « ses » relations entrantes —
    /// et donnerait deux lectures contradictoires du **même** lien. Or ce qui
    /// intéresse le lecteur est invariant : quelle table porte la contrainte, et
    /// donc laquelle refusera l'écriture.
    #[test]
    fn la_fleche_dune_relation_ne_change_pas_de_sens_selon_l_onglet() {
        use oxyn_catalog::{ForeignKey, ForeignKeyTarget, IncomingForeignKey};

        let clients = CatalogPath::for_relation(None, Some("public"), "clients").expect("path");
        let commandes = CatalogPath::for_relation(None, Some("public"), "commandes").expect("path");
        let mut cache = CatalogCache::new();
        for chemin in [&clients, &commandes] {
            cache
                .set_relation(chemin, Relation::new("t", RelationKind::Table))
                .expect("relation");
        }

        // `commandes` porte la clé vers `clients`.
        let cle = ForeignKey::new(
            "commandes_client_fk",
            vec!["client_id".into()],
            ForeignKeyTarget {
                relation: clients.clone(),
                fields: vec!["id".into()],
            },
        );
        cache
            .set_foreign_keys(&commandes, vec![cle.clone()])
            .expect("sortantes");
        cache
            .set_incoming_foreign_keys(
                &clients,
                vec![IncomingForeignKey {
                    source: commandes.clone(),
                    key: cle,
                    source_unique: None,
                }],
            )
            .expect("entrantes");

        // Vue depuis `commandes` — ses relations sortantes.
        let sortante = selected_relationship(&cache, &commandes, ObjectTab::Relations, 0)
            .expect("une relation sortante");
        // Vue depuis `clients` — ses relations entrantes. Le même lien.
        let entrante = selected_relationship(&cache, &clients, ObjectTab::IncomingRelations, 0)
            .expect("une relation entrante");

        let attendu = "public.commandes → public.clients";
        assert!(sortante.starts_with(attendu), "{sortante}");
        assert!(
            entrante.starts_with(attendu),
            "le même lien, lu de l'autre bout, garde son sens : {entrante}"
        );
    }

    /// Le modèle de requête liée cite ses identifiants — I-10 vaut aussi pour
    /// un texte qui ne s'exécute pas.
    ///
    /// Une table nommée `"users"; DROP TABLE audit; --` est légale dans
    /// PostgreSQL. Sans citation, le modèle ouvert dans une console exécuterait
    /// la suppression au premier clic sur `Run` — et le fait qu'Oxyn n'ait rien
    /// lancé lui-même ne serait qu'une consolation.
    #[test]
    fn le_modele_de_requete_liee_cite_ses_identifiants() {
        use oxyn_catalog::{ForeignKey, ForeignKeyTarget};

        let hostile =
            CatalogPath::for_relation(None, Some("public"), "users\"; DROP TABLE audit; --")
                .expect("un nom hostile reste un nom légal");
        let cible = CatalogPath::for_relation(None, Some("public"), "clients").expect("path");
        let mut cache = CatalogCache::new();
        for chemin in [&hostile, &cible] {
            cache
                .set_relation(chemin, Relation::new("t", RelationKind::Table))
                .expect("relation");
        }
        cache
            .set_foreign_keys(
                &hostile,
                vec![ForeignKey::new(
                    "fk",
                    vec!["client_id".into()],
                    ForeignKeyTarget {
                        relation: cible,
                        fields: vec!["id".into()],
                    },
                )],
            )
            .expect("clé");

        let sql = related_row_query(
            &cache,
            &hostile,
            ObjectTab::Relations,
            0,
            SqlDialect::Postgres,
        )
        .expect("un modèle");

        // Le guillemet fermant est doublé : la citation ne se clôt pas par
        // surprise, et le `;` reste à l'intérieur du nom.
        assert!(
            sql.contains("\"users\"\"; DROP TABLE audit; --\""),
            "l'identifiant doit être cité : {sql}"
        );
        assert!(sql.contains("LIMIT 200"), "le modèle est borné : {sql}");
        // La valeur est laissée à saisir : Oxyn ne l'a pas.
        assert!(sql.trim_end().ends_with("LIMIT 200;"), "{sql}");
    }

    /// Une clé composite donne autant de conditions que de colonnes.
    ///
    /// En omettre une rendrait le modèle silencieusement **plus large** que la
    /// relation qu'il prétend suivre — et l'utilisateur lirait des lignes qui
    /// n'ont rien à voir.
    #[test]
    fn une_cle_composite_donne_toutes_ses_conditions() {
        use oxyn_catalog::{ForeignKey, ForeignKeyTarget};

        let source = CatalogPath::for_relation(None, Some("public"), "lignes").expect("path");
        let cible = CatalogPath::for_relation(None, Some("public"), "commandes").expect("path");
        let mut cache = CatalogCache::new();
        for chemin in [&source, &cible] {
            cache
                .set_relation(chemin, Relation::new("t", RelationKind::Table))
                .expect("relation");
        }
        cache
            .set_foreign_keys(
                &source,
                vec![ForeignKey::new(
                    "fk",
                    vec!["commande_id".into(), "ligne_no".into()],
                    ForeignKeyTarget {
                        relation: cible,
                        fields: vec!["id".into(), "no".into()],
                    },
                )],
            )
            .expect("clé");

        let sql = related_row_query(
            &cache,
            &source,
            ObjectTab::Relations,
            0,
            SqlDialect::Postgres,
        )
        .expect("un modèle");
        assert!(sql.contains("\"commande_id\" = "), "{sql}");
        assert!(sql.contains("AND \"ligne_no\" = "), "{sql}");
    }
}
