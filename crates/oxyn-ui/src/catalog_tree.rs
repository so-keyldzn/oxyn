//! L'arbre de catalogue, paresseux et virtualisé.
//!
//! # Pourquoi paresseux
//!
//! Introspecter un schéma à 20 000 objets prend des minutes
//! ([ARCHITECTURE §6](../../../docs/ARCHITECTURE.md)). L'arbre ne demande donc
//! jamais un palier avant que l'utilisateur ne l'ouvre, et il ne demande jamais
//! deux fois ce que le cache connaît déjà. Un nœud ouvert dont le cache a la
//! réponse s'affiche sans aucun aller-retour : c'est le budget de 50 ms de
//! [PERFORMANCE](../../../docs/PERFORMANCE.md#budgets-dinteraction).
//!
//! # Pourquoi une liste plate
//!
//! L'arbre est **aplati** en [`TreeRow`] à chaque changement de structure, puis
//! rendu par `uniform_list`. Un espace de noms à 20 000 tables déplié ne
//! construit alors que la trentaine de lignes visibles. Une récursion à
//! l'affichage construirait les 20 000.
//!
//! # Le verrou n'est jamais attendu
//!
//! Le cache est partagé avec la tâche qui l'alimente. L'aplatissement passe par
//! [`RwLock::try_read`](parking_lot::RwLock::try_read) : si l'écrivain tient le
//! verrou, l'arbre garde les lignes de la trame précédente et réessaie. Le
//! thread d'interface n'attend **jamais** un verrou tenu par une tâche
//! ([I-05](../../../CLAUDE.md#i-05)) ; une trame en retard ne se voit pas, un
//! gel se voit.
//!
//! # Ce qui reste
//!
//! TODO(phase 1) : le menu contextuel (copier le nom qualifié, ouvrir les
//! données, voir le DDL) attend les commandes correspondantes — une vue ne
//! propose pas une action qu'aucune `Command` n'exprime
//! ([ADR-0004](../../../docs/adr/0004-command-bus.md)).

use std::collections::BTreeSet;
use std::ops::Range;

use gpui::prelude::*;
use gpui::{
    AnyElement, App, ClickEvent, Context, ElementId, EventEmitter, FocusHandle, Focusable,
    KeyDownEvent, MouseButton, MouseDownEvent, SharedString, UniformListScrollHandle, Window, div,
    px, uniform_list,
};
use oxyn_catalog::{
    CatalogCache, CatalogLevel, CatalogPath, CatalogScope, RelationKind, SearchOptions,
    SharedCatalog, search,
};
use oxyn_core::Capabilities;

use crate::icons::{IconName, icon};
use crate::text_field::{FieldEvent, TextField};
use crate::theme::Theme;

/// Résultats affichés au plus quand un filtre est saisi.
///
/// Au-delà, la liste n'aide plus à choisir : elle demande de choisir une
/// seconde fois.
pub const FILTER_RESULT_LIMIT: usize = 200;

/// Une ligne de l'arbre aplati.
#[derive(Debug, Clone, PartialEq)]
#[non_exhaustive]
pub struct TreeRow {
    /// Chemin du nœud. C'est aussi son identité pour le pliage.
    pub path: CatalogPath,
    /// Ce qui est écrit sur la ligne.
    pub label: SharedString,
    /// Mention à droite : nature, ou nombre de lignes estimé.
    pub detail: Option<SharedString>,
    /// Profondeur, pour le retrait.
    pub depth: usize,
    /// Palier occupé.
    pub level: CatalogLevel,
    /// Nature, pour les relations seulement.
    pub kind: Option<RelationKind>,
    /// Le nœud peut-il être déplié ?
    pub expandable: bool,
    /// Le nœud est-il déplié ?
    pub expanded: bool,
    /// Une lecture est-elle en cours pour ce nœud ?
    pub loading: bool,
}

/// Ce que l'arbre demande, sans jamais le faire lui-même.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum CatalogTreeEvent {
    /// Le contenu de ce sous-arbre doit être lu.
    ///
    /// Devient une [`Command::RefreshCatalog`](oxyn_core::Command) dans
    /// `oxyn-app` ; l'arbre n'appelle aucun `CatalogProvider`
    /// ([I-01](../../../CLAUDE.md#i-01)).
    ExpandRequested(CatalogScope),
    /// Le nœud sélectionné a changé.
    SelectionChanged(CatalogPath),
    /// L'utilisateur veut voir le contenu de cette relation.
    ///
    /// Émis seulement quand [`RelationKind::holds_records`] est vrai : proposer
    /// « ouvrir les données » d'une séquence serait une surface qui ne mène
    /// nulle part ([ADR-0003](../../../docs/adr/0003-driver-capabilities.md)).
    RelationActivated(CatalogPath),
}

/// L'arbre de catalogue d'une connexion.
#[derive(Debug)]
pub struct CatalogTree {
    focus: FocusHandle,
    catalog: SharedCatalog,
    capabilities: Capabilities,
    /// Nœuds dépliés. Un `BTreeSet` et non un `HashSet` : l'ordre stable rend
    /// les tests lisibles, et l'ensemble reste petit — ce sont les nœuds
    /// **ouverts**, pas les nœuds connus.
    expanded: BTreeSet<CatalogPath>,
    /// Nœuds dont l'expansion a été demandée et dont la réponse n'est pas
    /// arrivée.
    loading: BTreeSet<CatalogPath>,
    selected: Option<CatalogPath>,
    filter: String,
    filter_input: gpui::Entity<TextField>,
    rows: Vec<TreeRow>,
    scroll: UniformListScrollHandle,
}

impl EventEmitter<CatalogTreeEvent> for CatalogTree {}

impl Focusable for CatalogTree {
    fn focus_handle(&self, _cx: &App) -> FocusHandle {
        self.focus.clone()
    }
}

impl CatalogTree {
    /// Un arbre sur un cache partagé.
    ///
    /// `capabilities` sont celles de la **session**, pas du driver : une même
    /// implémentation PostgreSQL n'a pas les mêmes capacités selon la version
    /// du serveur et les extensions installées.
    pub fn new(
        catalog: SharedCatalog,
        capabilities: Capabilities,
        cx: &mut Context<'_, Self>,
    ) -> Self {
        let filter_input = cx.new(|cx| TextField::new(String::new(), false, cx));
        cx.subscribe(&filter_input, |this, input, event, cx| {
            if matches!(event, FieldEvent::Changed) {
                this.filter = input.read(cx).text().to_owned();
                this.rebuild();
                cx.notify();
            }
        })
        .detach();
        let mut arbre = Self {
            focus: cx.focus_handle(),
            catalog,
            capabilities,
            expanded: BTreeSet::new(),
            loading: BTreeSet::new(),
            selected: None,
            filter: String::new(),
            filter_input,
            rows: Vec::new(),
            scroll: UniformListScrollHandle::new(),
        };
        arbre.rebuild();
        arbre
    }

    /// Les lignes affichées à cet instant.
    #[must_use]
    pub fn rows(&self) -> &[TreeRow] {
        &self.rows
    }

    /// Le nœud sélectionné.
    #[must_use]
    pub fn selected(&self) -> Option<&CatalogPath> {
        self.selected.as_ref()
    }

    /// Le filtre courant.
    #[must_use]
    pub fn filter(&self) -> &str {
        &self.filter
    }

    /// Change le filtre et reconstruit les lignes.
    pub fn set_filter(&mut self, filter: impl Into<String>, cx: &mut Context<'_, Self>) {
        self.filter = filter.into();
        self.filter_input
            .update(cx, |field, cx| field.set_text(self.filter.clone(), cx));
        self.rebuild();
        cx.notify();
    }

    /// Signale que le cache a changé — après une lecture, ou après un DDL.
    ///
    /// Efface l'indicateur de chargement des nœuds désormais connus : le laisser
    /// tourner sur un nœud déjà lu ferait croire à une lecture sans fin.
    pub fn on_catalog_updated(&mut self, cx: &mut Context<'_, Self>) {
        if let Some(cache) = self.catalog.try_read() {
            self.loading
                .retain(|chemin| !cache.freshness(&CatalogScope::of(chemin)).is_known());
        }
        self.rebuild();
        cx.notify();
    }

    /// Clears loading indicators after a failed or cancelled explicit request.
    pub fn finish_loading(&mut self, cx: &mut Context<'_, Self>) {
        self.loading.clear();
        self.rebuild();
        cx.notify();
    }

    /// Déplie ou replie un nœud.
    ///
    /// Le pliage est **local** : il n'attend rien du serveur, et c'est le seul
    /// cas où l'affichage optimiste est légitime
    /// ([UX-SPEC](../../../docs/UX-SPEC.md#ce-qui-nest-jamais-optimiste)).
    pub fn toggle(&mut self, path: &CatalogPath, cx: &mut Context<'_, Self>) {
        if self.expanded.remove(path) {
            self.rebuild();
            cx.notify();
            return;
        }
        self.expanded.insert(path.clone());

        // On ne redemande pas ce que le cache sait déjà : c'est ce qui rend la
        // navigation « locale » au sens de PERFORMANCE.
        let scope = CatalogScope::of(path);
        let connu = self
            .catalog
            .try_read()
            .is_some_and(|cache| cache.freshness(&scope).is_known());
        if !connu {
            self.loading.insert(path.clone());
            cx.emit(CatalogTreeEvent::ExpandRequested(scope));
        }
        self.rebuild();
        cx.notify();
    }

    /// Sélectionne un nœud.
    pub fn select(&mut self, path: CatalogPath, cx: &mut Context<'_, Self>) {
        cx.emit(CatalogTreeEvent::SelectionChanged(path.clone()));
        self.selected = Some(path);
        cx.notify();
    }

    /// Reconstruit les lignes à partir du cache.
    ///
    /// Ne bloque jamais : si le verrou est tenu, les lignes de la trame
    /// précédente sont conservées telles quelles.
    pub fn rebuild(&mut self) {
        let Some(cache) = self.catalog.try_read() else {
            // Compté et non ignoré : si cette trace apparaît à chaque trame,
            // c'est que la tâche d'introspection tient le verrou trop longtemps,
            // et l'arbre paraîtra figé.
            tracing::trace!("catalog tree: cache lock busy, keeping previous rows");
            return;
        };
        self.rows = if self.filter.trim().is_empty() {
            flatten(&cache, &self.expanded, &self.loading, self.capabilities)
        } else {
            filtered_rows(&cache, &self.filter, self.capabilities)
        };
    }

    fn row_index(&self, path: &CatalogPath) -> Option<usize> {
        self.rows.iter().position(|ligne| &ligne.path == path)
    }

    fn on_key(&mut self, event: &KeyDownEvent, _window: &mut Window, cx: &mut Context<'_, Self>) {
        let courant = self.selected.clone();
        match event.keystroke.key.as_str() {
            "down" => self.move_selection(1, cx),
            "up" => self.move_selection(-1, cx),
            "right" => {
                if let Some(chemin) = courant
                    && !self.expanded.contains(&chemin)
                {
                    self.toggle(&chemin, cx);
                }
            }
            "left" => {
                if let Some(chemin) = courant
                    && self.expanded.contains(&chemin)
                {
                    self.toggle(&chemin, cx);
                }
            }
            "enter" => {
                if let Some(chemin) = courant {
                    self.activate(&chemin, cx);
                }
            }
            _ => {}
        }
    }

    fn move_selection(&mut self, delta: isize, cx: &mut Context<'_, Self>) {
        if self.rows.is_empty() {
            return;
        }
        let courant = self
            .selected
            .as_ref()
            .and_then(|chemin| self.row_index(chemin));
        let index = match courant {
            None => 0,
            Some(index) => {
                let index = isize::try_from(index).unwrap_or(0);
                let dernier = isize::try_from(self.rows.len().saturating_sub(1)).unwrap_or(0);
                let vise = index.saturating_add(delta).clamp(0, dernier);
                usize::try_from(vise).unwrap_or(0)
            }
        };
        let Some(ligne) = self.rows.get(index) else {
            return;
        };
        let chemin = ligne.path.clone();
        self.scroll
            .scroll_to_item(index, gpui::ScrollStrategy::Center);
        self.select(chemin, cx);
    }

    /// Ouvre le contenu d'une relation, quand cela a un sens.
    fn activate(&mut self, path: &CatalogPath, cx: &mut Context<'_, Self>) {
        let Some(ligne) = self.rows.iter().find(|ligne| &ligne.path == path) else {
            return;
        };
        match ligne.kind {
            Some(kind) if kind.holds_records() => {
                cx.emit(CatalogTreeEvent::RelationActivated(path.clone()));
            }
            // Un nœud sans contenu consultable se déplie, il ne s'ouvre pas.
            _ => {
                let chemin = path.clone();
                self.toggle(&chemin, cx);
            }
        }
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Aplatissement — logique pure, testable sans fenêtre
// ─────────────────────────────────────────────────────────────────────────────

/// Aplatit le cache en lignes, en ne descendant que dans les nœuds dépliés.
///
/// Les paliers absents sont **sautés**, pas simulés : MySQL n'a pas de
/// catalogue, Elasticsearch n'a ni catalogue ni espace de noms, et l'arbre ne
/// fabrique pas de niveau vide pour uniformiser
/// ([ARCHITECTURE §6](../../../docs/ARCHITECTURE.md)).
#[must_use]
pub fn flatten(
    cache: &CatalogCache,
    expanded: &BTreeSet<CatalogPath>,
    loading: &BTreeSet<CatalogPath>,
    capabilities: Capabilities,
) -> Vec<TreeRow> {
    let mut lignes = Vec::new();
    let catalogues: Vec<_> = cache.catalogs().cloned().collect();

    if catalogues.is_empty() {
        push_namespaces(&mut lignes, cache, None, 0, expanded, loading, capabilities);
        return lignes;
    }

    for catalogue in catalogues {
        let chemin = catalogue.path();
        let ouvert = expanded.contains(&chemin);
        lignes.push(TreeRow {
            label: SharedString::from(catalogue.name().to_owned()),
            detail: None,
            depth: 0,
            level: CatalogLevel::Catalog,
            kind: None,
            expandable: true,
            expanded: ouvert,
            loading: loading.contains(&chemin),
            path: chemin.clone(),
        });
        if ouvert {
            push_namespaces(
                &mut lignes,
                cache,
                Some(catalogue.name()),
                1,
                expanded,
                loading,
                capabilities,
            );
        }
    }
    lignes
}

fn push_namespaces(
    lignes: &mut Vec<TreeRow>,
    cache: &CatalogCache,
    catalogue: Option<&str>,
    depth: usize,
    expanded: &BTreeSet<CatalogPath>,
    loading: &BTreeSet<CatalogPath>,
    capabilities: Capabilities,
) {
    let espaces: Vec<_> = cache.namespaces(catalogue).cloned().collect();

    if espaces.is_empty() {
        // Ni catalogue ni espace de noms : les relations sont à la racine.
        let racine = match catalogue {
            None => CatalogPath::empty(),
            Some(nom) => match CatalogPath::for_catalog(nom) {
                Ok(chemin) => chemin,
                // Un nom refusé par `CatalogPath` vient du serveur ; on ignore
                // le sous-arbre plutôt que de paniquer (I-09).
                Err(_) => return,
            },
        };
        push_relations(lignes, cache, &racine, depth, capabilities);
        return;
    }

    for espace in espaces {
        let chemin = espace.path();
        let ouvert = expanded.contains(&chemin);
        lignes.push(TreeRow {
            label: SharedString::from(espace.name().to_owned()),
            detail: espace
                .is_system
                .then(|| SharedString::new_static("système")),
            depth,
            level: CatalogLevel::Namespace,
            kind: None,
            expandable: true,
            expanded: ouvert,
            loading: loading.contains(&chemin),
            path: chemin.clone(),
        });
        if ouvert {
            push_relations(
                lignes,
                cache,
                &chemin,
                depth.saturating_add(1),
                capabilities,
            );
        }
    }
}

fn push_relations(
    lignes: &mut Vec<TreeRow>,
    cache: &CatalogCache,
    namespace: &CatalogPath,
    depth: usize,
    capabilities: Capabilities,
) {
    for relation in cache.relations(namespace) {
        let chemin = relation.path();
        // Le nombre de lignes n'est affiché que si le driver déclare savoir
        // l'estimer : une estimation absente vaut mieux qu'un chiffre inventé.
        let detail = capabilities
            .contains(Capabilities::ROW_COUNT_ESTIMATE)
            .then(|| cache.relation(&chemin).and_then(|r| r.estimated_rows))
            .flatten()
            .map(|lignes| SharedString::from(format!("~{lignes}")));
        lignes.push(TreeRow {
            label: SharedString::from(relation.name().to_owned()),
            detail,
            depth,
            level: CatalogLevel::Relation,
            kind: Some(relation.kind),
            expandable: false,
            expanded: false,
            loading: false,
            path: chemin,
        });
    }
}

/// Les lignes correspondant à un filtre saisi.
///
/// Délègue à [`fn@oxyn_catalog::search`] : une seconde implémentation de la
/// correspondance divergerait de celle qu'utilisent les agents, et l'utilisateur
/// verrait deux classements différents pour la même saisie.
#[must_use]
pub fn filtered_rows(
    cache: &CatalogCache,
    filter: &str,
    capabilities: Capabilities,
) -> Vec<TreeRow> {
    let options = SearchOptions::default().with_limit(FILTER_RESULT_LIMIT);
    search(cache, filter, &options)
        .into_iter()
        .map(|hit| {
            let detail = capabilities
                .contains(Capabilities::ROW_COUNT_ESTIMATE)
                .then(|| cache.relation(&hit.path).and_then(|r| r.estimated_rows))
                .flatten()
                .map(|lignes| SharedString::from(format!("~{lignes}")))
                // À défaut d'estimation, montrer où le terme a répondu : le
                // résultat est autrement indistinguable des autres.
                .or_else(|| Some(SharedString::from(qualified_parent(&hit.path))));
            TreeRow {
                label: SharedString::from(hit.path.leaf().unwrap_or_default().to_owned()),
                detail,
                // Une recherche rend une liste, pas un arbre : tout est au même
                // niveau, et le chemin complet est la mention de droite.
                depth: 0,
                level: CatalogLevel::Relation,
                kind: Some(hit.kind),
                expandable: false,
                expanded: false,
                loading: false,
                path: hit.path,
            }
        })
        .collect()
}

/// Le chemin du parent, **écrit pour être lu et non pour être exécuté**.
///
/// Les paliers sont joints à la main plutôt que par `Display` : celui de
/// [`CatalogPath`] garde ses points de fin (`app.public.`) pour que l'aller-retour
/// avec `FromStr` soit exact, ce qui est juste pour un fichier de workspace et
/// laid dans une liste.
///
/// Aucun identifiant n'est cité ici, et c'est volontaire : cette chaîne ne part
/// jamais dans du SQL. La citation est le travail de
/// [`CatalogPath::qualify_sql`], et confondre les deux est exactement ce que
/// [I-10](../../../CLAUDE.md#i-10) interdit.
fn qualified_parent(path: &CatalogPath) -> String {
    match path.parent() {
        Some(parent) if !parent.is_empty() => parent.segments().collect::<Vec<_>>().join("."),
        _ => String::new(),
    }
}

/// Le glyphe qui distingue une nature de relation.
///
/// Une lettre et non une icône bitmap : GPUI n'embarque pas de jeu d'icônes, et
/// livrer des SVG pour la phase 0 coûterait plus que la lisibilité gagnée.
/// TODO(phase 1) : jeu d'icônes vectorielles, débloqué par le choix d'une
/// famille d'icônes libre.
#[must_use]
pub fn icon_for_kind(kind: RelationKind) -> &'static str {
    match kind {
        RelationKind::Table => "▤",
        RelationKind::View => "◇",
        RelationKind::MaterializedView => "◈",
        RelationKind::Collection => "❐",
        RelationKind::Index => "⌗",
        RelationKind::Stream => "≋",
        RelationKind::KeyPattern => "⚿",
        RelationKind::NodeLabel => "◉",
        RelationKind::RelationshipType => "→",
        RelationKind::Function => "ƒ",
        RelationKind::Procedure => "⚙",
        RelationKind::Sequence => "#",
        // `RelationKind` est `#[non_exhaustive]` : une nature ajoutée plus tard
        // se dessine plutôt que d'empêcher la compilation d'une crate d'interface.
        _ => "•",
    }
}

/// Le glyphe d'un nœud pliable.
#[must_use]
pub fn icon_for_node(expanded: bool, loading: bool) -> &'static str {
    if loading {
        "◌"
    } else if expanded {
        "▾"
    } else {
        "▸"
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Rendu
// ─────────────────────────────────────────────────────────────────────────────

impl Render for CatalogTree {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<'_, Self>) -> impl IntoElement {
        let theme = Theme::of(cx).clone();
        let lignes = self.rows.len();

        let corps = if lignes == 0 {
            self.render_empty(cx)
        } else {
            uniform_list(
                "oxyn-catalog-rows",
                lignes,
                cx.processor(|arbre: &mut Self, plage: Range<usize>, _window, cx| {
                    let theme = Theme::of(cx).clone();
                    let mut sorties = Vec::with_capacity(plage.len());
                    for index in plage {
                        // Cloner la ligne avant de rendre : `render_row` prend
                        // `&self`, et emprunter `arbre.rows` pendant l'appel
                        // interdirait tout accès au reste de l'état.
                        let Some(ligne) = arbre.rows.get(index).cloned() else {
                            continue;
                        };
                        sorties.push(arbre.render_row(&ligne, &theme, cx));
                    }
                    sorties
                }),
            )
            .track_scroll(self.scroll.clone())
            .size_full()
            .into_any_element()
        };

        div()
            .key_context("CatalogTree")
            .track_focus(&self.focus)
            .tab_index(0)
            .border_1()
            .border_color(theme.colors.surface)
            .focus(|style| style.border_color(theme.colors.border_focus))
            .id("oxyn-catalog-tree")
            .size_full()
            .flex()
            .flex_col()
            .bg(theme.colors.surface)
            .text_color(theme.colors.text)
            .font_family(theme.typography.ui_family.clone())
            .text_size(theme.typography.ui_size)
            .on_key_down(cx.listener(Self::on_key))
            .child(
                div()
                    .text_size(theme.typography.small_size)
                    .text_color(theme.colors.text_muted)
                    .px_2()
                    .child("Filter loaded objects"),
            )
            .child(self.render_filter(cx))
            .child(div().flex_1().overflow_hidden().child(corps))
    }
}

impl CatalogTree {
    /// Native input over cached objects only; never issues a remote search.
    fn render_filter(&self, cx: &Context<'_, Self>) -> AnyElement {
        let theme = Theme::of(cx);
        div()
            .flex()
            .items_center()
            .gap_2()
            .flex_none()
            .h(px(32.))
            .px_2()
            .border_b_1()
            .border_color(theme.colors.border)
            .child(
                icon(IconName::Search)
                    .size(px(16.))
                    .text_color(theme.colors.text_muted),
            )
            .child(div().flex_1().min_w_0().child(self.filter_input.clone()))
            .into_any_element()
    }

    /// L'état vide — celui qu'on oublie, et le premier que voit un nouvel
    /// utilisateur ([UX-SPEC](../../../docs/UX-SPEC.md#états-dune-vue)).
    fn render_empty(&self, cx: &Context<'_, Self>) -> AnyElement {
        let theme = Theme::of(cx);
        let (titre, detail) = if self.filter.trim().is_empty() {
            (
                "No objects",
                "The loaded catalog contains no visible objects.",
            )
        } else {
            (
                "Aucune correspondance",
                "Aucun objet connu ne correspond au filtre. Le filtre ne cherche que dans ce qui est déjà lu.",
            )
        };
        div()
            .flex_1()
            .flex()
            .flex_col()
            .gap_1()
            .items_center()
            .justify_center()
            .p_4()
            .text_size(theme.typography.small_size)
            .child(div().text_color(theme.colors.text_muted).child(titre))
            .child(div().text_color(theme.colors.text_faint).child(detail))
            .into_any_element()
    }

    fn render_row(&self, ligne: &TreeRow, theme: &Theme, cx: &Context<'_, Self>) -> AnyElement {
        let metrics = theme.metrics;
        let selectionnee = self.selected.as_ref() == Some(&ligne.path);
        // `u16` puis `f32::from` : `as` n'est pas nécessaire ici, et une
        // profondeur aberrante se borne au lieu de déborder en silence (I-09).
        let profondeur = u16::try_from(ligne.depth).unwrap_or(u16::MAX);
        let retrait = metrics.indent * f32::from(profondeur);
        let chemin_clic = ligne.path.clone();
        let chemin_bascule = ligne.path.clone();
        let ouvrable = ligne.expandable;

        div()
            .id(ElementId::Name(SharedString::from(format!(
                "oxyn-catalog-{}",
                ligne.path
            ))))
            .h(metrics.row_height)
            .w_full()
            .flex()
            .flex_row()
            .items_center()
            .gap_1()
            .pl(retrait)
            .pr_2()
            .when(selectionnee, |element| element.bg(theme.colors.selection))
            .hover(|style| style.bg(theme.colors.hover))
            .cursor_pointer()
            .on_click(cx.listener(move |arbre, event: &ClickEvent, window, cx| {
                window.focus(&arbre.focus);
                let chemin = chemin_clic.clone();
                // Un double-clic ouvre les données ; un simple clic sélectionne.
                if event.click_count() >= 2 {
                    arbre.select(chemin.clone(), cx);
                    arbre.activate(&chemin, cx);
                } else {
                    arbre.select(chemin, cx);
                }
            }))
            .child(
                div()
                    .w(px(16.0))
                    .flex_none()
                    .text_color(theme.colors.text_faint)
                    // `on_mouse_down` et non `on_click` : `on_click` exige un
                    // identifiant d'élément, et en poser un ici changerait le
                    // type de la branche `when`, qui doit rendre le même `Div`.
                    .when(ouvrable, |element| {
                        element.cursor_pointer().on_mouse_down(
                            MouseButton::Left,
                            cx.listener(move |arbre, _event: &MouseDownEvent, _window, cx| {
                                arbre.toggle(&chemin_bascule, cx);
                            }),
                        )
                    })
                    .when(ouvrable, |el| {
                        el.child(
                            icon(if ligne.expanded {
                                IconName::Down
                            } else {
                                IconName::Chevron
                            })
                            .size(px(16.))
                            .text_color(theme.colors.text_muted),
                        )
                    }),
            )
            .child(
                div()
                    .w(px(16.0))
                    .flex_none()
                    .text_color(theme.colors.text_muted)
                    .child(
                        icon(match ligne.level {
                            CatalogLevel::Namespace => IconName::Folder,
                            CatalogLevel::Relation => IconName::Table,
                            _ => IconName::Database,
                        })
                        .size(px(16.))
                        .text_color(theme.colors.text_muted),
                    ),
            )
            .child(
                div()
                    .flex_1()
                    .overflow_hidden()
                    .whitespace_nowrap()
                    .text_ellipsis()
                    .child(ligne.label.clone()),
            )
            .when_some(ligne.detail.clone(), |element, detail| {
                element.child(
                    div()
                        .flex_none()
                        .text_size(theme.typography.small_size)
                        .text_color(theme.colors.text_faint)
                        .child(detail),
                )
            })
            .into_any_element()
    }
}

#[cfg(test)]
mod tests {
    use oxyn_catalog::{CatalogRef, NamespaceRef, Relation, RelationRef};

    use super::*;

    fn cache_postgres() -> CatalogCache {
        let mut cache = CatalogCache::new();
        cache.set_catalogs(vec![
            CatalogRef::new("app").expect("nom de catalogue valide"),
        ]);
        let catalogue = CatalogPath::for_catalog("app").expect("chemin valide");
        cache.set_namespaces(
            Some("app"),
            vec![
                NamespaceRef::new(catalogue.clone(), "public").expect("nom valide"),
                NamespaceRef::new(catalogue, "pg_catalog")
                    .expect("nom valide")
                    .with_system(),
            ],
        );
        let public = CatalogPath::for_namespace(Some("app"), "public").expect("chemin valide");
        cache
            .set_relations(
                &public,
                vec![
                    RelationRef::new(public.clone(), "clients", RelationKind::Table)
                        .expect("nom valide"),
                    RelationRef::new(public.clone(), "v_actifs", RelationKind::View)
                        .expect("nom valide"),
                ],
            )
            .expect("public est un espace de noms");
        cache
    }

    #[test]
    fn larbre_ne_descend_pas_dans_un_noeud_replie() {
        let cache = cache_postgres();
        let lignes = flatten(
            &cache,
            &BTreeSet::new(),
            &BTreeSet::new(),
            Capabilities::empty(),
        );
        // Un seul catalogue, replié : rien d'autre n'est construit. C'est toute
        // la raison d'être de l'arbre paresseux.
        assert_eq!(lignes.len(), 1);
        assert_eq!(lignes[0].label, "app");
        assert!(lignes[0].expandable);
        assert!(!lignes[0].expanded);
    }

    #[test]
    fn deplier_un_catalogue_montre_ses_espaces_de_noms() {
        let cache = cache_postgres();
        let mut ouverts = BTreeSet::new();
        ouverts.insert(CatalogPath::for_catalog("app").expect("chemin valide"));
        let lignes = flatten(&cache, &ouverts, &BTreeSet::new(), Capabilities::empty());
        assert_eq!(lignes.len(), 3);
        assert_eq!(lignes[1].level, CatalogLevel::Namespace);
        assert_eq!(lignes[1].depth, 1);
        assert_eq!(
            lignes[2].detail.as_ref().map(SharedString::as_str),
            Some("système")
        );
    }

    #[test]
    fn deplier_un_espace_de_noms_montre_ses_relations_avec_leur_nature() {
        let cache = cache_postgres();
        let mut ouverts = BTreeSet::new();
        ouverts.insert(CatalogPath::for_catalog("app").expect("chemin valide"));
        ouverts.insert(CatalogPath::for_namespace(Some("app"), "public").expect("chemin valide"));
        let lignes = flatten(&cache, &ouverts, &BTreeSet::new(), Capabilities::empty());
        let relations: Vec<_> = lignes
            .iter()
            .filter(|ligne| ligne.level == CatalogLevel::Relation)
            .collect();
        assert_eq!(relations.len(), 2);
        assert_eq!(relations[0].kind, Some(RelationKind::Table));
        assert_eq!(relations[1].kind, Some(RelationKind::View));
        assert_eq!(relations[0].depth, 2);
        assert!(!relations[0].expandable);
    }

    #[test]
    fn un_serveur_sans_catalogue_expose_ses_espaces_de_noms_a_la_racine() {
        // MySQL : pas de palier catalogue. L'arbre ne fabrique pas un niveau
        // vide pour uniformiser (ARCHITECTURE §6).
        let mut cache = CatalogCache::new();
        cache.set_namespaces(
            None,
            vec![NamespaceRef::new(CatalogPath::empty(), "boutique").expect("nom valide")],
        );
        let lignes = flatten(
            &cache,
            &BTreeSet::new(),
            &BTreeSet::new(),
            Capabilities::empty(),
        );
        assert_eq!(lignes.len(), 1);
        assert_eq!(lignes[0].level, CatalogLevel::Namespace);
        assert_eq!(lignes[0].depth, 0);
    }

    #[test]
    fn un_serveur_sans_palier_intermediaire_expose_ses_relations_a_la_racine() {
        // Elasticsearch : ni catalogue ni espace de noms.
        let mut cache = CatalogCache::new();
        let racine = CatalogPath::empty();
        cache
            .set_relations(
                &racine,
                vec![
                    RelationRef::new(racine.clone(), "logs-2026", RelationKind::Index)
                        .expect("nom valide"),
                ],
            )
            .expect("la racine n'est pas une relation");
        let lignes = flatten(
            &cache,
            &BTreeSet::new(),
            &BTreeSet::new(),
            Capabilities::empty(),
        );
        assert_eq!(lignes.len(), 1);
        assert_eq!(lignes[0].kind, Some(RelationKind::Index));
    }

    #[test]
    fn le_nombre_de_lignes_estime_nest_montre_que_si_le_driver_le_sait() {
        // ADR-0003 : rien n'est simulé. Sans la capacité, pas de chiffre.
        let mut cache = cache_postgres();
        let clients =
            CatalogPath::for_relation(Some("app"), Some("public"), "clients").expect("chemin");
        cache
            .set_relation(
                &clients,
                Relation::new("clients", RelationKind::Table).with_estimated_rows(4_200),
            )
            .expect("la relation existe dans le cache");

        let mut ouverts = BTreeSet::new();
        ouverts.insert(CatalogPath::for_catalog("app").expect("chemin valide"));
        ouverts.insert(CatalogPath::for_namespace(Some("app"), "public").expect("chemin valide"));

        let sans = flatten(&cache, &ouverts, &BTreeSet::new(), Capabilities::empty());
        let ligne = sans
            .iter()
            .find(|ligne| ligne.label == "clients")
            .expect("la relation est dans l'arbre");
        assert_eq!(ligne.detail, None);

        let avec = flatten(
            &cache,
            &ouverts,
            &BTreeSet::new(),
            Capabilities::ROW_COUNT_ESTIMATE,
        );
        let ligne = avec
            .iter()
            .find(|ligne| ligne.label == "clients")
            .expect("la relation est dans l'arbre");
        assert_eq!(
            ligne.detail.as_ref().map(SharedString::as_str),
            Some("~4200")
        );
    }

    #[test]
    fn lindicateur_de_chargement_suit_le_noeud_demande() {
        let cache = cache_postgres();
        let catalogue = CatalogPath::for_catalog("app").expect("chemin valide");
        let mut en_cours = BTreeSet::new();
        en_cours.insert(catalogue.clone());
        let lignes = flatten(&cache, &BTreeSet::new(), &en_cours, Capabilities::empty());
        assert!(lignes[0].loading);
        assert_eq!(icon_for_node(false, true), "◌");
    }

    #[test]
    fn le_filtre_rend_une_liste_plate_de_relations() {
        let cache = cache_postgres();
        let lignes = filtered_rows(&cache, "client", Capabilities::empty());
        assert_eq!(lignes.len(), 1);
        assert_eq!(lignes[0].label, "clients");
        assert_eq!(lignes[0].depth, 0);
        assert_eq!(
            lignes[0].detail.as_ref().map(SharedString::as_str),
            Some("app.public"),
            "le chemin affiché ne garde pas les points de fin de `Display`"
        );
    }

    #[test]
    fn un_filtre_sans_correspondance_rend_une_liste_vide() {
        let cache = cache_postgres();
        assert!(filtered_rows(&cache, "zzzz", Capabilities::empty()).is_empty());
    }

    #[test]
    fn chaque_nature_de_relation_a_un_glyphe_propre() {
        let natures = [
            RelationKind::Table,
            RelationKind::View,
            RelationKind::MaterializedView,
            RelationKind::Collection,
            RelationKind::Index,
            RelationKind::Stream,
            RelationKind::KeyPattern,
            RelationKind::NodeLabel,
            RelationKind::RelationshipType,
            RelationKind::Function,
            RelationKind::Procedure,
            RelationKind::Sequence,
        ];
        let glyphes: BTreeSet<_> = natures.iter().map(|k| icon_for_kind(*k)).collect();
        assert_eq!(glyphes.len(), natures.len(), "deux natures se confondent");
    }
    #[gpui::test]
    fn native_filter_changes_visible_objects_without_requesting_the_server(
        cx: &mut gpui::TestAppContext,
    ) {
        let cache = std::sync::Arc::new(parking_lot::RwLock::new(cache_postgres()));
        let (tree, cx) =
            cx.add_window_view(|_, cx| CatalogTree::new(cache, Capabilities::TABLES, cx));
        cx.update(|window, cx| {
            window.focus(&tree.read(cx).filter_input.read(cx).focus_handle(cx));
        });
        cx.simulate_input("zzzz");
        tree.read_with(cx, |tree, _| {
            assert_eq!(tree.filter(), "zzzz");
            assert!(tree.rows().is_empty());
            assert!(
                tree.loading.is_empty(),
                "local filtering must not request introspection"
            );
        });
        cx.simulate_keystrokes("cmd-a");
        cx.simulate_input("clients");
        tree.read_with(cx, |tree, _| {
            assert_eq!(tree.filter(), "clients");
            assert!(
                !tree.rows().is_empty(),
                "matching cached objects must reappear"
            );
            assert!(tree.loading.is_empty());
        });
    }
}
