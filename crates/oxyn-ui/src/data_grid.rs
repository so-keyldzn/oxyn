//! La grille de résultats virtualisée.
//!
//! C'est le composant sur lequel se gagne ou se perd la promesse du produit :
//! « `SELECT` de 10 M de lignes, premier affichage sous 100 ms, mémoire stable »
//! ([IMPLEMENTATION-PLAN](../../../docs/IMPLEMENTATION-PLAN.md), phase 0).
//!
//! # Les quatre règles qui gouvernent ce fichier
//!
//! **Rien n'est matérialisé.** La grille lit un [`ResultBuffer`] partagé et
//! appelle [`format_cell`] sur les seules cellules visibles. Aucune structure
//! `Vec<Vec<String>>` n'existe, ni pour tout le résultat, ni pour une page.
//! Convertir « pour simplifier l'affichage » annulerait l'accès O(1) d'Arrow et
//! réintroduirait exactement l'empreinte mémoire qu'[ADR-0002] élimine.
//! Le seul texte alloué par trame est celui des cellules réellement dessinées,
//! parce que GPUI ne sait pas dessiner un `&str` emprunté.
//!
//! **Rien n'attend la fin.** Le nombre de lignes est relu à chaque trame sur le
//! tampon. La grille se dessine dès que le schéma existe, se remplit au fur et
//! à mesure des lots, et l'utilisateur fait défiler pendant que la requête
//! tourne.
//!
//! **Le thread d'interface ne lit pas le disque** ([I-05](../../../CLAUDE.md#i-05)).
//! Un lot qui a débordé sur disque n'est pas rechargé pendant le rendu : la
//! ligne est dessinée en attente. La réhydratation en tâche de fond est
//! `// TODO(phase 1)` — voir [`row_batch`].
//!
//! **Les colonnes sont virtualisées comme les lignes.** Une table à 300
//! colonnes ne construit pas 300 éléments par ligne : [`visible_columns`]
//! découpe la plage horizontale, et c'est une fonction pure, testée.
//!
//! # Les cinq états ([UX-SPEC](../../../docs/UX-SPEC.md#états-dune-vue))
//!
//! | État | [`GridState`] |
//! |---|---|
//! | Initial | [`GridState::Idle`] |
//! | En cours | [`GridState::Starting`], ou [`GridState::Streaming`] tant que le tampon n'est pas clos |
//! | Peuplé | [`GridState::Streaming`] clos, avec des lignes |
//! | Vide | [`GridState::Streaming`] clos, sans ligne |
//! | Erreur | [`GridState::Failed`] |
//!
//! [ADR-0002]: ../../../docs/adr/0002-arrow-result-model.md

use std::cell::Cell;
use std::ops::Range;
use std::rc::Rc;
use std::sync::Arc;

use arrow::datatypes::{DataType, SchemaRef};
use arrow::record_batch::RecordBatch;
use gpui::prelude::*;
use gpui::{
    AnyElement, App, Bounds, ClickEvent, Context, ElementId, EventEmitter, FocusHandle, Focusable,
    Hsla, KeyDownEvent, MouseButton, MouseDownEvent, MouseMoveEvent, MouseUpEvent, Pixels,
    ScrollStrategy, ScrollWheelEvent, SharedString, UniformListScrollHandle, Window, canvas, div,
    px, uniform_list,
};
use oxyn_core::Capabilities;
use oxyn_data::cell::{CellValue, FormatOptions, format_cell};
use oxyn_data::{BatchIndex, ResultBuffer};

use crate::controls::{ControlState, ControlTone, control};
use crate::session_capabilities::cancel_caveat;
use crate::theme::{Metrics, Theme};

/// Lignes échantillonnées pour estimer la largeur des colonnes.
///
/// Cent lignes suffisent à distinguer une colonne d'identifiants d'une colonne
/// de commentaires ; en mesurer dix mille coûterait le budget d'affichage du
/// premier lot, c'est-à-dire précisément ce qu'on cherche à protéger.
pub const WIDTH_SAMPLE_ROWS: usize = 100;

/// Clé de métadonnée Arrow marquant un champ dont le type a été **déduit**.
///
/// Portée par le `Field` Arrow, elle permet à la grille de dire que le schéma
/// vient d'un échantillonnage (Mongo, documents JSON) et non du serveur —
/// exigence d'[ADR-0002].
///
/// TODO(phase 3) : `DRIVER-CONTRACT` doit inscrire cette clé au contrat, pour
/// que les drivers documentaires la posent. Tant qu'aucun driver ne la pose,
/// la grille ne l'affiche jamais — elle ne l'invente pas.
///
/// [ADR-0002]: ../../../docs/adr/0002-arrow-result-model.md
pub const INFERRED_FIELD_KEY: &str = "oxyn.inferred";

/// Une colonne, telle que la grille la dessine.
#[derive(Debug, Clone, PartialEq)]
#[non_exhaustive]
pub struct ColumnLayout {
    /// Nom de la colonne, tel que le serveur l'a donné.
    pub name: SharedString,
    /// Type, en abrégé lisible.
    pub type_name: SharedString,
    /// Largeur courante.
    pub width: Pixels,
    /// La colonne accepte-t-elle l'absence de valeur ?
    pub nullable: bool,
    /// Le type a-t-il été déduit par échantillonnage plutôt que déclaré ?
    pub inferred: bool,
}

impl ColumnLayout {
    /// Largeur d'en-tête suffisante pour le nom et le type.
    fn header_width(&self, metrics: &Metrics) -> Pixels {
        let caracteres = self
            .name
            .chars()
            .count()
            .max(self.type_name.chars().count());
        text_width(caracteres, metrics) + metrics.cell_padding * 2.0
    }
}

/// Les colonnes visibles à un décalage horizontal donné.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VisibleColumns {
    /// Indices des colonnes à construire.
    pub range: Range<usize>,
    /// Largeur cumulée des colonnes situées avant [`range`](Self::range).
    ///
    /// La ligne est décalée de `leading - offset` : la soustraction est
    /// toujours négative ou nulle, ce qui fait glisser le contenu sous le bord
    /// gauche sans jamais laisser de trou.
    pub leading: Pixels,
    /// Largeur cumulée de toutes les colonnes.
    pub total: Pixels,
}

/// L'état d'une exécution, du point de vue de la grille.
///
/// Une énumération et non un assemblage de booléens : « en cours » et « échoué »
/// ne peuvent pas être vrais ensemble, et deux drapeaux indépendants finissent
/// toujours par l'être.
#[derive(Debug, Clone)]
#[non_exhaustive]
pub enum GridState {
    /// Rien n'a encore été exécuté. La grille explique quoi faire.
    Idle,
    /// Une exécution est lancée, le schéma n'est pas encore connu.
    Starting,
    /// Execution was cancelled before any result was available.
    Cancelled,
    /// Le schéma est connu. Les lignes arrivent, ou sont toutes arrivées :
    /// [`ResultBuffer::is_complete`] tranche, et la grille n'a pas à le savoir
    /// autrement.
    Streaming(Arc<ResultBuffer>),
    /// L'exécution a échoué.
    Failed {
        /// Le message **du serveur**, code compris.
        ///
        /// Pas une paraphrase : le public d'Oxyn lit les erreurs de PostgreSQL
        /// ([UX-SPEC](../../../docs/UX-SPEC.md#les-erreurs-sadressent-à-un-professionnel)).
        message: SharedString,
        /// L'opération peut-elle être relancée telle quelle ?
        ///
        /// Vient de [`ErrorClass`](oxyn_core::ErrorClass), jamais d'une
        /// analyse du message ([I-13](../../../CLAUDE.md#i-13)).
        retryable: bool,
    },
}

impl GridState {
    /// Le tampon, quand il existe.
    #[must_use]
    pub fn buffer(&self) -> Option<&Arc<ResultBuffer>> {
        match self {
            Self::Streaming(tampon) => Some(tampon),
            Self::Idle | Self::Starting | Self::Cancelled | Self::Failed { .. } => None,
        }
    }

    /// Une exécution est-elle en cours ?
    ///
    /// Un tampon non clos compte : c'est ce qui fait apparaître le moyen
    /// d'annuler, et l'absence de ce bouton est le défaut que
    /// [UX-SPEC](../../../docs/UX-SPEC.md#annulation) interdit.
    #[must_use]
    pub fn is_running(&self) -> bool {
        match self {
            Self::Starting => true,
            Self::Streaming(tampon) => !tampon.is_complete(),
            Self::Idle | Self::Cancelled | Self::Failed { .. } => false,
        }
    }
}

/// Ce que la grille demande, sans jamais le faire elle-même.
///
/// Une vue ne parle pas à un driver : elle émet, et `oxyn-app` traduit en
/// [`Command`](oxyn_core::Command) ([I-01](../../../CLAUDE.md#i-01)).
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum GridEvent {
    /// L'utilisateur a sélectionné une ligne.
    RowSelected(usize),
    /// L'utilisateur demande l'annulation de l'exécution en cours.
    CancelRequested,
}

/// Un redimensionnement de colonne en cours.
#[derive(Debug, Clone, Copy, PartialEq)]
struct ColumnDrag {
    column: usize,
    origin_x: Pixels,
    origin_width: Pixels,
}

/// La grille de résultats.
#[derive(Debug)]
pub struct DataGrid {
    focus: FocusHandle,
    state: GridState,
    columns: Vec<ColumnLayout>,
    /// Vrai tant que les largeurs n'ont pas été ajustées sur un lot réel.
    widths_measured: bool,
    format: FormatOptions,
    scroll: UniformListScrollHandle,
    /// Décalage horizontal, en pixels depuis le bord gauche des colonnes.
    h_offset: Pixels,
    /// Largeur utile mesurée à la trame précédente.
    ///
    /// Partagée avec la fermeture de mesure du `canvas` : c'est le seul moyen
    /// de connaître la taille réelle sans refaire la mise en page à la main.
    viewport: Rc<Cell<Pixels>>,
    selected_row: Option<usize>,
    drag: Option<ColumnDrag>,
    /// What the connected session declares. Governs what the cancel control is
    /// allowed to promise ([ADR-0003](../../../docs/adr/0003-driver-capabilities.md)).
    capabilities: Capabilities,
}

impl EventEmitter<GridEvent> for DataGrid {}

impl Focusable for DataGrid {
    fn focus_handle(&self, _cx: &App) -> FocusHandle {
        self.focus.clone()
    }
}

impl DataGrid {
    /// Une grille vide, à l'état initial.
    pub fn new(cx: &mut Context<'_, Self>) -> Self {
        Self {
            focus: cx.focus_handle(),
            state: GridState::Idle,
            columns: Vec::new(),
            widths_measured: false,
            format: FormatOptions::default(),
            scroll: UniformListScrollHandle::new(),
            h_offset: Pixels::ZERO,
            viewport: Rc::new(Cell::new(px(0.0))),
            selected_row: None,
            drag: None,
            // Empty until a session is connected: promising nothing is the safe
            // default, promising server-side cancellation is not.
            capabilities: Capabilities::empty(),
        }
    }

    /// Declares what the connected session can do.
    pub fn set_capabilities(&mut self, capabilities: Capabilities, cx: &mut Context<'_, Self>) {
        if self.capabilities != capabilities {
            self.capabilities = capabilities;
            cx.notify();
        }
    }

    /// What the session declares.
    #[must_use]
    pub const fn capabilities(&self) -> Capabilities {
        self.capabilities
    }

    /// L'état courant.
    #[must_use]
    pub fn state(&self) -> &GridState {
        &self.state
    }

    /// Les colonnes courantes.
    #[must_use]
    pub fn columns(&self) -> &[ColumnLayout] {
        &self.columns
    }

    /// La ligne sélectionnée.
    #[must_use]
    pub fn selected_row(&self) -> Option<usize> {
        self.selected_row
    }

    /// Les réglages de rendu des cellules.
    #[must_use]
    pub fn format_options(&self) -> &FormatOptions {
        &self.format
    }

    /// Change les réglages de rendu des cellules.
    pub fn set_format_options(&mut self, options: FormatOptions, cx: &mut Context<'_, Self>) {
        self.format = options;
        cx.notify();
    }

    /// Signale qu'une exécution vient d'être lancée.
    ///
    /// Efface le résultat précédent : afficher les lignes de la requête d'avant
    /// pendant que la suivante tourne est le mensonge le plus facile à commettre
    /// et le plus difficile à repérer.
    pub fn start(&mut self, cx: &mut Context<'_, Self>) {
        self.state = GridState::Starting;
        self.columns.clear();
        self.widths_measured = false;
        self.selected_row = None;
        self.h_offset = Pixels::ZERO;
        cx.notify();
    }

    /// Shows cancellation without presenting it as a permanent server failure.
    pub fn cancelled(&mut self, cx: &mut Context<'_, Self>) {
        self.state = GridState::Cancelled;
        cx.notify();
    }

    /// Attache le tampon dès que le schéma est connu.
    ///
    /// À appeler à la réception de [`Event::SchemaReady`](oxyn_core::Event),
    /// **avant** le premier lot : les en-têtes s'affichent immédiatement, ce qui
    /// est déjà un retour visible dans le budget des 300 ms
    /// ([PERFORMANCE](../../../docs/PERFORMANCE.md#budgets-dinteraction)).
    pub fn set_buffer(&mut self, buffer: Arc<ResultBuffer>, cx: &mut Context<'_, Self>) {
        if self
            .state
            .buffer()
            .is_some_and(|current| Arc::ptr_eq(current, &buffer))
        {
            cx.notify();
            return;
        }
        let metrics = Theme::of(cx).metrics;
        self.columns = column_layouts(buffer.schema(), &metrics);
        // Le nombre de colonnes, jamais leurs noms : un nom de colonne peut
        // porter de l'information sur les données de l'utilisateur, et un
        // journal n'est pas le bon endroit pour cela (I-03).
        tracing::debug!(columns = self.columns.len(), "grid: schema attached");
        self.widths_measured = false;
        self.selected_row = None;
        self.h_offset = Pixels::ZERO;
        self.state = GridState::Streaming(buffer);
        cx.notify();
    }

    /// Signale l'arrivée d'un lot.
    ///
    /// Le tampon étant partagé, la grille n'a rien à recopier ; l'appel sert à
    /// redessiner et, la première fois, à ajuster les largeurs sur des valeurs
    /// réelles plutôt que sur les seuls en-têtes.
    pub fn on_batch(&mut self, cx: &mut Context<'_, Self>) {
        if !self.widths_measured
            && let Some(tampon) = self.state.buffer().cloned()
            && let Some(lot) = first_resident_batch(&tampon)
        {
            let metrics = Theme::of(cx).metrics;
            fit_columns(
                &mut self.columns,
                &lot,
                &self.format,
                &metrics,
                WIDTH_SAMPLE_ROWS,
            );
            self.widths_measured = true;
        }
        cx.notify();
    }

    /// Signale l'échec de l'exécution.
    pub fn fail(
        &mut self,
        message: impl Into<SharedString>,
        retryable: bool,
        cx: &mut Context<'_, Self>,
    ) {
        self.state = GridState::Failed {
            message: message.into(),
            retryable,
        };
        cx.notify();
    }

    /// Remet la grille à l'état initial.
    pub fn reset(&mut self, cx: &mut Context<'_, Self>) {
        self.state = GridState::Idle;
        self.columns.clear();
        self.widths_measured = false;
        self.selected_row = None;
        self.h_offset = Pixels::ZERO;
        cx.notify();
    }

    /// Impose la largeur d'une colonne, bornée par [`Metrics::min_column_width`].
    ///
    /// Sans effet si l'indice ne désigne aucune colonne : un indice hors borne
    /// vient d'un événement de souris, donc d'une entrée, et une entrée ne
    /// panique pas ([I-09](../../../CLAUDE.md#i-09)).
    pub fn set_column_width(&mut self, column: usize, width: Pixels, cx: &mut Context<'_, Self>) {
        let minimum = Theme::of(cx).metrics.min_column_width;
        if let Some(colonne) = self.columns.get_mut(column) {
            colonne.width = width.max(minimum);
            cx.notify();
        }
    }

    /// Nombre de lignes reçues à cet instant.
    fn row_count(&self) -> usize {
        self.state.buffer().map_or(0, |tampon| tampon.row_count())
    }

    /// Largeur réellement disponible pour les colonnes.
    ///
    /// La mesure porte sur toute la grille ; la gouttière de numéros de ligne
    /// est figée et n'appartient pas à la piste défilante, il faut donc la
    /// retrancher. L'oublier ferait construire une colonne de trop à droite —
    /// invisible à l'écran, mais payée à chaque trame.
    fn content_viewport(&self, metrics: &Metrics) -> Pixels {
        let utile = f32::from(self.viewport.get()) - f32::from(metrics.gutter_width);
        px(utile.max(0.0))
    }

    /// Décale le contenu horizontalement, en restant dans les bornes.
    ///
    /// Rend `true` si le décalage a changé — pour ne pas redessiner sur un
    /// défilement purement vertical, qui produit aussi des événements dont la
    /// composante horizontale est nulle.
    fn scroll_horizontally(&mut self, delta: Pixels, metrics: &Metrics) -> bool {
        let total: f32 = self
            .columns
            .iter()
            .map(|colonne| f32::from(colonne.width))
            .sum();
        let maximum = (total - f32::from(self.content_viewport(metrics))).max(0.0);
        let vise = (f32::from(self.h_offset) + f32::from(delta)).clamp(0.0, maximum);
        if (vise - f32::from(self.h_offset)).abs() < f32::EPSILON {
            return false;
        }
        self.h_offset = px(vise);
        true
    }

    /// Déplace la sélection de `delta` lignes et fait défiler jusqu'à elle.
    fn move_selection(&mut self, delta: isize, cx: &mut Context<'_, Self>) {
        let lignes = self.row_count();
        if lignes == 0 {
            return;
        }
        // `try_from` et non `as` : un nombre de lignes vient du serveur, et un
        // `as` qui déborde est silencieux ([I-09](../../../CLAUDE.md#i-09)).
        let courante = isize::try_from(self.selected_row.unwrap_or(0)).unwrap_or(isize::MAX);
        let dernier = isize::try_from(lignes.saturating_sub(1)).unwrap_or(isize::MAX);
        let visee = courante.saturating_add(delta).clamp(0, dernier);
        let visee = usize::try_from(visee).unwrap_or(0);
        self.selected_row = Some(visee);
        self.scroll.scroll_to_item(visee, ScrollStrategy::Center);
        cx.emit(GridEvent::RowSelected(visee));
        cx.notify();
    }

    fn on_key(&mut self, event: &KeyDownEvent, _window: &mut Window, cx: &mut Context<'_, Self>) {
        let metrics = Theme::of(cx).metrics;
        let pas_colonne = f32::from(metrics.default_column_width);
        let lignes_par_page = 20;
        match event.keystroke.key.as_str() {
            "up" => self.move_selection(-1, cx),
            "down" => self.move_selection(1, cx),
            "pageup" => self.move_selection(-lignes_par_page, cx),
            "pagedown" => self.move_selection(lignes_par_page, cx),
            "home" => self.move_selection(isize::MIN / 2, cx),
            "end" => self.move_selection(isize::MAX / 2, cx),
            "left" => {
                if self.scroll_horizontally(px(-pas_colonne), &metrics) {
                    cx.notify();
                }
            }
            "right" => {
                if self.scroll_horizontally(px(pas_colonne), &metrics) {
                    cx.notify();
                }
            }
            // Échap n'annule que pendant une exécution : sur une grille au
            // repos, il ne doit rien faire du tout.
            "escape" if self.state.is_running() => cx.emit(GridEvent::CancelRequested),
            _ => {}
        }
    }

    fn on_wheel(
        &mut self,
        event: &ScrollWheelEvent,
        _window: &mut Window,
        cx: &mut Context<'_, Self>,
    ) {
        let metrics = Theme::of(cx).metrics;
        let delta = event.delta.pixel_delta(metrics.row_height);
        // Le défilement vertical est celui de `uniform_list` ; seule la
        // composante horizontale nous concerne, et l'inversion de signe est
        // celle d'un contenu qui glisse sous une fenêtre fixe.
        if self.scroll_horizontally(px(-f32::from(delta.x)), &metrics) {
            cx.notify();
        }
    }

    fn on_drag_move(
        &mut self,
        event: &MouseMoveEvent,
        _window: &mut Window,
        cx: &mut Context<'_, Self>,
    ) {
        let Some(drag) = self.drag else {
            return;
        };
        if event.pressed_button != Some(MouseButton::Left) {
            self.drag = None;
            return;
        }
        let ecart = f32::from(event.position.x) - f32::from(drag.origin_x);
        let largeur = px(f32::from(drag.origin_width) + ecart);
        self.set_column_width(drag.column, largeur, cx);
    }

    fn on_drag_end(
        &mut self,
        _event: &MouseUpEvent,
        _window: &mut Window,
        _cx: &mut Context<'_, Self>,
    ) {
        self.drag = None;
    }

    /// La plage de colonnes à construire pour cette trame.
    fn visible_columns(&self, metrics: &Metrics) -> VisibleColumns {
        visible_columns(&self.columns, self.h_offset, self.content_viewport(metrics))
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Logique pure — testable sans fenêtre, et c'est là que sont les erreurs
// intéressantes.
// ─────────────────────────────────────────────────────────────────────────────

/// Les colonnes déduites d'un schéma Arrow, dimensionnées sur leurs en-têtes.
///
/// Appelée dès que le schéma est connu, donc avant tout lot : c'est ce qui
/// permet de dessiner les en-têtes sans attendre une ligne.
#[must_use]
pub fn column_layouts(schema: &SchemaRef, metrics: &Metrics) -> Vec<ColumnLayout> {
    schema
        .fields()
        .iter()
        .map(|champ| {
            let mise = ColumnLayout {
                name: SharedString::from(champ.name().clone()),
                type_name: SharedString::from(short_type_name(champ.data_type())),
                width: metrics.default_column_width,
                nullable: champ.is_nullable(),
                inferred: champ.metadata().contains_key(INFERRED_FIELD_KEY),
            };
            let largeur = mise
                .header_width(metrics)
                .max(metrics.min_column_width)
                .min(metrics.max_column_width);
            ColumnLayout {
                width: largeur,
                ..mise
            }
        })
        .collect()
}

/// Élargit les colonnes sur un échantillon de valeurs réelles.
///
/// La mesure est une **estimation** en caractères ×
/// [`Metrics::char_width`] : mesurer réellement le texte demanderait de mettre
/// en page cent lignes hors écran, ce qui coûterait le budget du premier
/// affichage pour un gain que l'utilisateur corrige d'un glissement.
/// Elle n'élargit jamais, ne rétrécit jamais en dessous de l'en-tête, et reste
/// bornée par [`Metrics::max_column_width`].
pub fn fit_columns(
    columns: &mut [ColumnLayout],
    batch: &RecordBatch,
    options: &FormatOptions,
    metrics: &Metrics,
    sample_rows: usize,
) {
    let lignes = batch.num_rows().min(sample_rows);
    for (index, colonne) in columns.iter_mut().enumerate() {
        let mut caracteres = 0usize;
        for ligne in 0..lignes {
            let valeur = format_cell(batch, ligne, index, options);
            let longueur = match &valeur {
                CellValue::Null => options.null_text.chars().count(),
                CellValue::Text(texte) => texte.chars().count(),
                // Une valeur coupée est déjà à la longueur d'affichage : la
                // mesurer entière ferait une colonne large de tout le document.
                CellValue::Truncated { text, .. } => text.chars().count(),
                CellValue::Unrenderable { reason } => reason.chars().count(),
                // `#[non_exhaustive]` : voir `cell_element`. Une variante
                // inconnue s'y rend en « unsupported value », dont c'est ici la
                // largeur — mesurer autre chose couperait le texte affiché.
                _ => "unsupported value".chars().count(),
            };
            caracteres = caracteres.max(longueur);
        }
        if caracteres == 0 {
            continue;
        }
        let voulue = text_width(caracteres, metrics) + metrics.cell_padding * 2.0;
        let bornee = voulue
            .max(colonne.header_width(metrics))
            .max(metrics.min_column_width)
            .min(metrics.max_column_width);
        colonne.width = colonne.width.max(bornee).min(metrics.max_column_width);
    }
}

/// Les colonnes intersectant la fenêtre `[offset, offset + viewport]`.
///
/// **Chemin chaud** : appelée une fois par trame, elle décide combien
/// d'éléments GPUI existent par ligne. Sans elle, une table à 300 colonnes
/// construirait 300 éléments par ligne visible — soit 12 000 éléments par trame
/// pour n'en montrer que 400.
///
/// Une largeur de fenêtre nulle (première trame, avant toute mesure) rend une
/// plage vide plutôt que toutes les colonnes : la trame suivante corrige, et
/// dessiner trois cents colonnes une fois vaut mieux évité.
#[must_use]
pub fn visible_columns(
    columns: &[ColumnLayout],
    offset: Pixels,
    viewport: Pixels,
) -> VisibleColumns {
    let mut cumul = 0.0f32;
    let debut_fenetre = f32::from(offset).max(0.0);
    let fin_fenetre = debut_fenetre + f32::from(viewport).max(0.0);

    let mut premier = None;
    let mut dernier = 0usize;
    let mut avant = 0.0f32;

    for (index, colonne) in columns.iter().enumerate() {
        let debut = cumul;
        let fin = cumul + f32::from(colonne.width);
        cumul = fin;

        if fin <= debut_fenetre {
            avant = fin;
            continue;
        }
        if debut >= fin_fenetre {
            continue;
        }
        if premier.is_none() {
            premier = Some(index);
            avant = debut;
        }
        dernier = index;
    }

    match premier {
        Some(debut) => VisibleColumns {
            range: debut..dernier.saturating_add(1),
            leading: px(avant),
            total: px(cumul),
        },
        None => VisibleColumns {
            range: 0..0,
            leading: Pixels::ZERO,
            total: px(cumul),
        },
    }
}

/// Le lot contenant `row`, **seulement s'il est en mémoire**.
///
/// Rend `None` pour un lot débordé sur disque : le relire bloquerait le thread
/// d'interface le temps d'une lecture de page, ce qu'[I-05] interdit. La grille
/// dessine alors une ligne en attente.
///
/// TODO(phase 1) : demander la réhydratation en tâche de fond au lieu de laisser
/// la ligne en attente jusqu'à ce que l'utilisateur repasse dessus. Débloqué par
/// l'ordonnanceur d'`oxyn-exec`, qui est le seul à pouvoir tenir la poignée de
/// la tâche.
///
/// [I-05]: ../../../CLAUDE.md#i-05
#[must_use]
pub fn row_batch(buffer: &ResultBuffer, row: usize) -> Option<(RecordBatch, usize)> {
    let (position, decalage) = buffer.locate(row)?;
    if !buffer.is_resident(position) {
        return None;
    }
    // `batch` peut encore échouer (relecture, schéma) ; un échec de lecture
    // n'est pas une raison de paniquer pendant un rendu.
    buffer
        .batch(position)
        .ok()
        .flatten()
        .map(|lot| (lot, decalage))
}

/// Le premier lot encore en mémoire, pour l'échantillon de mesure.
fn first_resident_batch(buffer: &ResultBuffer) -> Option<RecordBatch> {
    let position = BatchIndex::new(0);
    if !buffer.is_resident(position) {
        return None;
    }
    buffer.batch(position).ok().flatten()
}

/// Largeur estimée de `caracteres` caractères à chasse fixe.
fn text_width(caracteres: usize, metrics: &Metrics) -> Pixels {
    // `u16` puis `f32::from` : pas de `as`, et un décompte aberrant se borne
    // plutôt que de déborder en silence (I-09). Le plafond de 65 535 caractères
    // est très au-delà de `Metrics::max_column_width`, qui tranche ensuite.
    let compte = u16::try_from(caracteres).unwrap_or(u16::MAX);
    metrics.char_width * f32::from(compte)
}

/// Nom court d'un type Arrow, tel que l'en-tête l'affiche.
///
/// `Debug` d'`arrow` écrit `Timestamp(Microsecond, Some("UTC"))`, illisible dans
/// une colonne de 120 pixels. Les types composés gardent leur forme `Debug`,
/// faute de mieux, et sont coupés à l'affichage.
fn short_type_name(data_type: &DataType) -> String {
    match data_type {
        DataType::Null => "null".into(),
        DataType::Boolean => "bool".into(),
        DataType::Int8 => "int8".into(),
        DataType::Int16 => "int16".into(),
        DataType::Int32 => "int32".into(),
        DataType::Int64 => "int64".into(),
        DataType::UInt8 => "uint8".into(),
        DataType::UInt16 => "uint16".into(),
        DataType::UInt32 => "uint32".into(),
        DataType::UInt64 => "uint64".into(),
        DataType::Float16 => "float16".into(),
        DataType::Float32 => "float32".into(),
        DataType::Float64 => "float64".into(),
        DataType::Utf8 | DataType::LargeUtf8 | DataType::Utf8View => "text".into(),
        DataType::Binary | DataType::LargeBinary | DataType::BinaryView => "bytes".into(),
        DataType::Date32 | DataType::Date64 => "date".into(),
        DataType::Time32(_) | DataType::Time64(_) => "time".into(),
        DataType::Timestamp(_, zone) => match zone {
            Some(_) => "timestamptz".into(),
            None => "timestamp".into(),
        },
        DataType::Interval(_) | DataType::Duration(_) => "interval".into(),
        DataType::Decimal128(precision, echelle) | DataType::Decimal256(precision, echelle) => {
            format!("decimal({precision},{echelle})")
        }
        DataType::List(_) | DataType::LargeList(_) | DataType::FixedSizeList(_, _) => "list".into(),
        DataType::Struct(_) => "struct".into(),
        DataType::Map(_, _) => "map".into(),
        DataType::Dictionary(_, valeur) => short_type_name(valeur),
        autre => format!("{autre:?}"),
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Rendu
// ─────────────────────────────────────────────────────────────────────────────

impl Render for DataGrid {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<'_, Self>) -> impl IntoElement {
        let theme = Theme::of(cx).clone();
        let viewport = Rc::clone(&self.viewport);
        let entity = cx.entity_id();
        let racine = div()
            .relative()
            .child(
                canvas(
                    move |bounds: Bounds<Pixels>, _, cx: &mut App| {
                        if viewport.replace(bounds.size.width) != bounds.size.width {
                            // Notify after layout so the next frame uses the measured width.
                            cx.defer(move |cx| cx.notify(entity));
                        }
                    },
                    |_, (), _, _| {},
                )
                .absolute()
                .inset_0(),
            )
            .key_context("DataGrid")
            .track_focus(&self.focus)
            .id("oxyn-data-grid")
            .border_1()
            .border_color(theme.colors.background)
            .focus(|style| style.border_color(theme.colors.border_focus))
            .size_full()
            .flex()
            .flex_col()
            .bg(theme.colors.background)
            .text_color(theme.colors.text)
            .font_family(theme.typography.mono_family.clone())
            .text_size(theme.typography.mono_size)
            .on_key_down(cx.listener(Self::on_key))
            .on_scroll_wheel(cx.listener(Self::on_wheel))
            .on_mouse_move(cx.listener(Self::on_drag_move))
            .on_mouse_up(MouseButton::Left, cx.listener(Self::on_drag_end));

        match &self.state {
            GridState::Idle => racine.child(self.render_placeholder(
                "Aucun résultat",
                "Écrivez une requête et exécutez-la avec Cmd+Entrée.",
                theme.colors.text_faint,
                cx,
            )),
            GridState::Starting => racine.child(self.render_placeholder(
                "Exécution…",
                "En attente du schéma. Échap annule.",
                theme.colors.text_muted,
                cx,
            )),
            GridState::Cancelled => racine.child(self.render_placeholder(
                "Exécution annulée",
                "Vous pouvez modifier ou relancer la requête.",
                theme.colors.text_muted,
                cx,
            )),
            GridState::Failed { message, retryable } => {
                let detail = if *retryable {
                    "L'erreur est transitoire : la requête peut être relancée telle quelle."
                } else {
                    "L'erreur est permanente : relancer la même requête donnera le même résultat."
                };
                racine.child(self.render_placeholder(
                    message.clone(),
                    detail,
                    theme.colors.danger,
                    cx,
                ))
            }
            GridState::Streaming(tampon) => {
                let tampon = Arc::clone(tampon);
                let lignes = tampon.row_count();
                if lignes == 0 && tampon.is_complete() {
                    return racine
                        .child(self.render_header(cx))
                        .child(self.render_placeholder(
                            "Aucune ligne",
                            "La requête a abouti et n'a renvoyé aucune ligne.",
                            theme.colors.text_muted,
                            cx,
                        ))
                        .into_any_element();
                }
                racine
                    .child(self.render_header(cx))
                    .child(self.render_rows(lignes, cx))
                    .child(self.render_footer(&tampon, cx))
            }
        }
        .into_any_element()
    }
}

impl DataGrid {
    /// Un état sans lignes : initial, en cours, vide ou erreur.
    fn render_placeholder(
        &self,
        titre: impl Into<SharedString>,
        detail: impl Into<SharedString>,
        couleur: Hsla,
        cx: &Context<'_, Self>,
    ) -> AnyElement {
        let theme = Theme::of(cx);
        let annulable = self.state.is_running();
        div()
            .flex_1()
            .flex()
            .flex_col()
            .gap_2()
            .items_center()
            .justify_center()
            .p_6()
            .child(
                div()
                    .text_color(couleur)
                    .font_family(theme.typography.ui_family.clone())
                    .text_size(theme.typography.ui_size)
                    .child(titre.into()),
            )
            .child(
                div()
                    .text_color(theme.colors.text_faint)
                    .font_family(theme.typography.ui_family.clone())
                    .text_size(theme.typography.small_size)
                    .child(detail.into()),
            )
            // L'état « en cours » porte toujours son moyen d'annuler
            // (UX-SPEC) ; l'annulation atteint le serveur, c'est `oxyn-exec`
            // qui s'en charge à la réception de l'événement.
            .when(annulable, |element| {
                element.child(self.render_cancel_button(cx))
            })
            .into_any_element()
    }

    /// Le bouton d'annulation.
    ///
    /// Il émet ; il n'annule pas. L'annulation réelle est le travail
    /// d'`oxyn-exec`, qui la propage jusqu'au serveur quand la capacité
    /// `SERVER_SIDE_CANCEL` est là. Un bouton qui abandonnerait seulement
    /// l'affichage laisserait une requête tourner et une connexion prise
    /// ([UX-SPEC](../../../docs/UX-SPEC.md#annulation)).
    ///
    /// Quand la session ne déclare pas `SERVER_SIDE_CANCEL`, le bouton reste —
    /// il coupe bien le flux — mais il cesse de promettre ce qu'il ne tient
    /// pas : la mention l'accompagne, elle ne le remplace pas.
    fn render_cancel_button(&self, cx: &Context<'_, Self>) -> AnyElement {
        let theme = Theme::of(cx);
        let Some(reserve) = cancel_caveat(self.capabilities) else {
            return self.cancel_control(cx);
        };
        div()
            .flex()
            .flex_col()
            .items_center()
            .gap_1()
            .child(self.cancel_control(cx))
            .child(
                div()
                    .text_color(theme.colors.warning)
                    .font_family(theme.typography.ui_family.clone())
                    .text_size(theme.typography.small_size)
                    .child(reserve),
            )
            .into_any_element()
    }

    /// Le bouton lui-même, sans la mention qui l'accompagne.
    fn cancel_control(&self, cx: &Context<'_, Self>) -> AnyElement {
        let theme = Theme::of(cx);
        control(
            "oxyn-grid-cancel",
            ControlState::Enabled,
            ControlTone::Neutral,
            theme,
            cx.listener(|_grille, _event: &ClickEvent, _window, cx| {
                cx.emit(GridEvent::CancelRequested);
            }),
        )
        .mt(theme.spacing.small)
        .px(theme.spacing.medium)
        .py(theme.spacing.tiny)
        .font_family(theme.typography.ui_family.clone())
        .text_size(theme.typography.small_size)
        .child("Annuler (Échap)")
        .into_any_element()
    }

    /// La ligne d'en-têtes, figée au-dessus des lignes.
    fn render_header(&self, cx: &Context<'_, Self>) -> AnyElement {
        let theme = Theme::of(cx);
        let metrics = theme.metrics;
        let visibles = self.visible_columns(&metrics);
        let decalage = f32::from(visibles.leading) - f32::from(self.h_offset);

        let mut piste = div()
            .flex()
            .flex_row()
            .flex_none()
            .ml(px(decalage))
            .h(metrics.header_height);

        for index in visibles.range.clone() {
            let Some(colonne) = self.columns.get(index) else {
                continue;
            };
            piste = piste.child(self.render_header_cell(index, colonne, cx));
        }

        div()
            .flex()
            .flex_row()
            .flex_none()
            .h(metrics.header_height)
            .bg(theme.colors.grid_header)
            .border_b_1()
            .border_color(theme.colors.grid_line)
            .child(
                // Coin de la gouttière : figé, aligné sur les numéros de ligne.
                div()
                    .w(metrics.gutter_width)
                    .h_full()
                    .flex_none()
                    .border_r_1()
                    .border_color(theme.colors.grid_line)
                    .bg(theme.colors.grid_gutter),
            )
            .child(div().flex_1().overflow_hidden().child(piste))
            .into_any_element()
    }

    fn render_header_cell(
        &self,
        index: usize,
        colonne: &ColumnLayout,
        cx: &Context<'_, Self>,
    ) -> AnyElement {
        let theme = Theme::of(cx);
        let metrics = theme.metrics;
        let largeur_origine = colonne.width;

        div()
            .w(colonne.width)
            .h_full()
            .flex_none()
            .flex()
            .flex_row()
            .items_center()
            .border_r_1()
            .border_color(theme.colors.grid_line)
            .child(
                div()
                    .flex_1()
                    .overflow_hidden()
                    .px(metrics.cell_padding)
                    .flex()
                    .flex_col()
                    .justify_center()
                    .child(
                        div()
                            .whitespace_nowrap()
                            .text_ellipsis()
                            .text_color(theme.colors.text)
                            .child(colonne.name.clone()),
                    )
                    .child(
                        div()
                            .whitespace_nowrap()
                            .text_ellipsis()
                            .text_size(theme.typography.small_size)
                            .text_color(if colonne.inferred {
                                theme.colors.warning
                            } else {
                                theme.colors.text_faint
                            })
                            .child(if colonne.inferred {
                                // ADR-0002 : un schéma déduit ne se présente
                                // jamais comme une vérité du serveur.
                                SharedString::from(format!("{} (déduit)", colonne.type_name))
                            } else {
                                colonne.type_name.clone()
                            }),
                    ),
            )
            .child(
                div()
                    .id(ElementId::named_usize("oxyn-col-resize", index))
                    .w(px(4.0))
                    .h_full()
                    .flex_none()
                    .cursor_col_resize()
                    .hover(|style| style.bg(theme.colors.accent))
                    .on_mouse_down(
                        MouseButton::Left,
                        cx.listener(
                            move |grille: &mut Self, event: &MouseDownEvent, _window, _cx| {
                                // Le glissement commence ici ; il est suivi par
                                // `on_drag_move`, posé sur la racine de la grille,
                                // parce qu'un déplacement de souris ne parvient
                                // qu'aux éléments survolés.
                                grille.drag = Some(ColumnDrag {
                                    column: index,
                                    origin_x: event.position.x,
                                    origin_width: largeur_origine,
                                });
                            },
                        ),
                    ),
            )
            .into_any_element()
    }

    /// La liste virtualisée. Seules les lignes visibles sont construites.
    fn render_rows(&self, lignes: usize, cx: &Context<'_, Self>) -> AnyElement {
        let liste = uniform_list(
            "oxyn-grid-rows",
            lignes,
            cx.processor(|grille: &mut Self, plage: Range<usize>, _window, cx| {
                grille.render_row_range(plage, cx)
            }),
        )
        .track_scroll(self.scroll.clone())
        .size_full();

        div()
            .flex_1()
            .relative()
            .overflow_hidden()
            .child(liste)
            .into_any_element()
    }

    /// Construit les lignes d'une plage — appelée par `uniform_list`.
    fn render_row_range(
        &mut self,
        plage: Range<usize>,
        cx: &mut Context<'_, Self>,
    ) -> Vec<AnyElement> {
        let theme = Theme::of(cx).clone();
        let visibles = self.visible_columns(&theme.metrics);
        let Some(tampon) = self.state.buffer().cloned() else {
            return Vec::new();
        };

        // Les lignes consécutives partagent presque toujours un lot : le garder
        // évite une recherche binaire et un clone d'`Arc` par ligne.
        let mut dernier: Option<(BatchIndex, RecordBatch)> = None;
        let mut lignes = Vec::with_capacity(plage.len());

        for ligne in plage {
            let cellules = match tampon.locate(ligne) {
                // La ligne n'est pas encore arrivée : la requête est en cours et
                // `uniform_list` demande au-delà de ce qui existe. Ce n'est pas
                // une erreur.
                None => None,
                Some((position, decalage)) => {
                    // L'emprunt sur `dernier` se termine ici, avant la
                    // réaffectation plus bas.
                    let connu = match &dernier {
                        Some((connue, lot)) if *connue == position => Some(lot.clone()),
                        _ => None,
                    };
                    let lot = match connu {
                        Some(lot) => Some(lot),
                        None => match row_batch(&tampon, ligne) {
                            Some((lot, _)) => {
                                dernier = Some((position, lot.clone()));
                                Some(lot)
                            }
                            None => None,
                        },
                    };
                    lot.map(|lot| (lot, decalage))
                }
            };
            lignes.push(self.render_row(ligne, cellules, &visibles, &theme, cx));
        }
        lignes
    }

    fn render_row(
        &self,
        ligne: usize,
        contenu: Option<(RecordBatch, usize)>,
        visibles: &VisibleColumns,
        theme: &Theme,
        cx: &Context<'_, Self>,
    ) -> AnyElement {
        let metrics = theme.metrics;
        let selectionnee = self.selected_row == Some(ligne);
        let decalage = f32::from(visibles.leading) - f32::from(self.h_offset);

        let mut piste = div()
            .flex()
            .flex_row()
            .flex_none()
            .ml(px(decalage))
            .h_full();

        for index in visibles.range.clone() {
            let Some(colonne) = self.columns.get(index) else {
                continue;
            };
            piste = piste.child(match &contenu {
                Some((lot, decalage_local)) => {
                    render_cell(lot, *decalage_local, index, colonne, &self.format, theme)
                }
                // Lot débordé sur disque : la ligne existe, sa valeur n'est pas
                // lisible sans I/O. On le dit, on ne dessine pas du vide.
                None => div()
                    .w(colonne.width)
                    .h_full()
                    .flex_none()
                    .px(metrics.cell_padding)
                    .flex()
                    .items_center()
                    .border_r_1()
                    .border_color(theme.colors.grid_line)
                    .text_color(theme.colors.text_faint)
                    .child("…")
                    .into_any_element(),
            });
        }

        div()
            .id(ElementId::named_usize("oxyn-grid-row", ligne))
            .h(metrics.row_height)
            .w_full()
            .flex()
            .flex_row()
            .items_center()
            .when(ligne % 2 == 1, |element| {
                element.bg(theme.colors.grid_stripe)
            })
            .when(selectionnee, |element| element.bg(theme.colors.selection))
            .hover(|style| style.bg(theme.colors.hover))
            .on_click(cx.listener(move |grille, _event: &ClickEvent, window, cx| {
                window.focus(&grille.focus);
                grille.selected_row = Some(ligne);
                cx.emit(GridEvent::RowSelected(ligne));
                cx.notify();
            }))
            .child(
                div()
                    .w(metrics.gutter_width)
                    .h_full()
                    .flex_none()
                    .flex()
                    .items_center()
                    .justify_end()
                    .px(metrics.cell_padding)
                    .bg(theme.colors.grid_gutter)
                    .border_r_1()
                    .border_color(theme.colors.grid_line)
                    .text_color(theme.colors.text_faint)
                    // Numérotation à partir de 1 : c'est ce que dit le serveur
                    // dans ses messages d'erreur, et ce que compte l'utilisateur.
                    .child(SharedString::from(ligne.saturating_add(1).to_string())),
            )
            .child(div().flex_1().h_full().overflow_hidden().child(piste))
            .into_any_element()
    }

    /// Le pied de grille : ce que le tampon sait de lui-même.
    fn render_footer(&self, tampon: &Arc<ResultBuffer>, cx: &Context<'_, Self>) -> AnyElement {
        let theme = Theme::of(cx);
        let stats = tampon.stats();
        let complet = tampon.is_complete();
        let libelle = if complet {
            format!("{} lignes", stats.rows)
        } else {
            format!("{} lignes reçues…", tampon.row_count())
        };

        div()
            .flex()
            .flex_row()
            .flex_none()
            .items_center()
            .gap_3()
            .h(theme.metrics.header_height)
            .px_3()
            .bg(theme.colors.surface_raised)
            .border_t_1()
            .border_color(theme.colors.grid_line)
            .font_family(theme.typography.ui_family.clone())
            .text_size(theme.typography.small_size)
            .text_color(theme.colors.text_muted)
            .child(SharedString::from(libelle))
            .when(stats.truncated, |element| {
                element.child(
                    div()
                        .text_color(theme.colors.warning)
                        .child("résultat tronqué"),
                )
            })
            .when(tampon.spilled_batches() > 0, |element| {
                element.child(
                    div()
                        .text_color(theme.colors.text_faint)
                        .child("débordé sur disque"),
                )
            })
            .when(!complet, |element| {
                element.child(self.render_cancel_button(cx))
            })
            .into_any_element()
    }
}

/// Une cellule. Le texte est formaté à la volée depuis le `RecordBatch`.
fn render_cell(
    lot: &RecordBatch,
    ligne: usize,
    colonne_index: usize,
    colonne: &ColumnLayout,
    options: &FormatOptions,
    theme: &Theme,
) -> AnyElement {
    let valeur = format_cell(lot, ligne, colonne_index, options);
    let base = div()
        .w(colonne.width)
        .h_full()
        .flex_none()
        .flex()
        .flex_row()
        .items_center()
        .gap_1()
        .px(theme.metrics.cell_padding)
        .overflow_hidden()
        .border_r_1()
        .border_color(theme.colors.grid_line);

    match valeur {
        // `NULL` n'est pas la chaîne « NULL » : italique et couleur propre, pour
        // qu'une colonne texte contenant littéralement « NULL » s'en distingue.
        CellValue::Null => base
            .italic()
            .text_color(theme.colors.null)
            .child(
                div()
                    .whitespace_nowrap()
                    .text_ellipsis()
                    .child(SharedString::from(options.null_text.to_string())),
            )
            .into_any_element(),
        CellValue::Text(texte) => base
            .child(
                div()
                    .whitespace_nowrap()
                    .text_ellipsis()
                    .child(SharedString::from(texte.into_owned())),
            )
            .into_any_element(),
        CellValue::Truncated { text, full_bytes } => base
            .child(
                div()
                    .flex_1()
                    .whitespace_nowrap()
                    .text_ellipsis()
                    .child(SharedString::from(text.into_owned())),
            )
            .child(
                // L'indicateur dit que la valeur est coupée à l'affichage, et
                // sa taille réelle : sans lui, l'utilisateur croit avoir tout vu.
                div()
                    .flex_none()
                    .px_1()
                    .rounded_sm()
                    .bg(theme.colors.surface_raised)
                    .text_size(theme.typography.small_size)
                    .text_color(theme.colors.warning)
                    .child(SharedString::from(format!("+{full_bytes} o"))),
            )
            .into_any_element(),
        // Un type qu'on ne sait pas rendre se dit ; il ne se déguise pas en
        // cellule vide, qui serait un mensonge sur des données réelles.
        CellValue::Unrenderable { reason } => base
            .text_color(theme.colors.danger)
            .child(
                div()
                    .whitespace_nowrap()
                    .text_ellipsis()
                    .child(SharedString::from(reason.into_owned())),
            )
            .into_any_element(),
        // `CellValue` est `#[non_exhaustive]` : une variante ajoutée dans
        // `oxyn-data` arrive ici sans casser la compilation. Elle se signale
        // pour la même raison qu'`Unrenderable` — afficher une cellule vide
        // mentirait sur des données réelles — et la trace nomme la crate à
        // corriger, parce que le défaut est ici, pas dans les données.
        autre => {
            tracing::error!(
                cell = ?autre,
                "unhandled CellValue variant: oxyn-ui is behind oxyn-data"
            );
            base.text_color(theme.colors.danger)
                .child(
                    div()
                        .whitespace_nowrap()
                        .text_ellipsis()
                        .child(SharedString::new_static("unsupported value")),
                )
                .into_any_element()
        }
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use arrow::array::{ArrayRef, Int64Array, StringArray};
    use arrow::datatypes::{DataType, Field, Schema};

    use super::*;

    fn colonnes(largeurs: &[f32]) -> Vec<ColumnLayout> {
        largeurs
            .iter()
            .enumerate()
            .map(|(index, largeur)| ColumnLayout {
                name: SharedString::from(format!("c{index}")),
                type_name: SharedString::new_static("text"),
                width: px(*largeur),
                nullable: true,
                inferred: false,
            })
            .collect()
    }

    #[test]
    fn la_fenetre_horizontale_ne_garde_que_les_colonnes_qui_la_coupent() {
        let colonnes = colonnes(&[100.0, 100.0, 100.0, 100.0, 100.0]);
        let visibles = visible_columns(&colonnes, px(150.0), px(200.0));
        // La fenêtre [150, 350) coupe les colonnes 1 [100,200), 2 [200,300)
        // et 3 [300,400).
        assert_eq!(visibles.range, 1..4);
        assert_eq!(visibles.leading, px(100.0));
        assert_eq!(visibles.total, px(500.0));
    }

    #[test]
    fn le_decalage_de_piste_est_toujours_negatif_ou_nul() {
        // C'est l'invariant qui empêche un trou blanc au bord gauche : la piste
        // est décalée de `leading - offset`, et `leading <= offset` par
        // construction.
        let colonnes = colonnes(&[80.0, 140.0, 60.0, 300.0]);
        for offset in [0.0, 25.0, 79.0, 80.0, 220.0, 400.0, 579.0] {
            let visibles = visible_columns(&colonnes, px(offset), px(200.0));
            assert!(
                f32::from(visibles.leading) <= offset,
                "leading {:?} dépasse l'offset {offset}",
                visibles.leading
            );
        }
    }

    #[test]
    fn une_fenetre_de_largeur_nulle_ne_construit_aucune_colonne() {
        // Première trame, avant toute mesure : mieux vaut une trame vide que
        // trois cents colonnes construites pour rien.
        let colonnes = colonnes(&[100.0; 300]);
        let visibles = visible_columns(&colonnes, Pixels::ZERO, Pixels::ZERO);
        assert!(visibles.range.is_empty());
        assert_eq!(visibles.total, px(30_000.0));
    }

    #[test]
    fn sans_colonne_la_plage_est_vide_et_le_total_nul() {
        let visibles = visible_columns(&[], Pixels::ZERO, px(800.0));
        assert!(visibles.range.is_empty());
        assert_eq!(visibles.total, Pixels::ZERO);
    }

    #[test]
    fn un_decalage_au_dela_du_contenu_ne_rend_aucune_colonne() {
        let colonnes = colonnes(&[100.0, 100.0]);
        let visibles = visible_columns(&colonnes, px(1_000.0), px(200.0));
        assert!(visibles.range.is_empty());
    }

    fn schema_exemple() -> SchemaRef {
        Arc::new(Schema::new(vec![
            Field::new("id", DataType::Int64, false),
            Field::new("libelle_tres_long_de_colonne", DataType::Utf8, true),
        ]))
    }

    #[test]
    fn les_colonnes_naissent_du_schema_sans_aucune_ligne() {
        // C'est ce qui permet de dessiner les en-têtes avant le premier lot.
        let metrics = Metrics::default();
        let colonnes = column_layouts(&schema_exemple(), &metrics);
        assert_eq!(colonnes.len(), 2);
        assert_eq!(colonnes[0].name, "id");
        assert_eq!(colonnes[0].type_name, "int64");
        assert!(!colonnes[0].nullable);
        assert!(colonnes[1].nullable);
        assert!(!colonnes[1].inferred);
    }

    #[test]
    fn la_largeur_dentete_couvre_le_nom_de_colonne() {
        let metrics = Metrics::default();
        let colonnes = column_layouts(&schema_exemple(), &metrics);
        assert!(colonnes[1].width > colonnes[0].width);
        assert!(colonnes[1].width <= metrics.max_column_width);
    }

    #[test]
    fn un_champ_marque_deduit_est_signale() {
        let mut champ = Field::new("payload", DataType::Utf8, true);
        champ = champ.with_metadata(
            [(INFERRED_FIELD_KEY.to_owned(), "true".to_owned())]
                .into_iter()
                .collect(),
        );
        let schema: SchemaRef = Arc::new(Schema::new(vec![champ]));
        let colonnes = column_layouts(&schema, &Metrics::default());
        assert!(colonnes[0].inferred);
    }

    fn lot_exemple() -> RecordBatch {
        let schema = schema_exemple();
        let ids = Int64Array::from(vec![1, 2, 3]);
        let libelles = StringArray::from(vec![
            Some("court"),
            None,
            Some("une valeur nettement plus longue que l'en-tête"),
        ]);
        let colonnes: Vec<ArrayRef> = vec![Arc::new(ids), Arc::new(libelles)];
        RecordBatch::try_new(schema, colonnes).expect("le lot d'exemple respecte son propre schéma")
    }

    #[test]
    fn la_mesure_elargit_sur_les_valeurs_et_reste_bornee() {
        let metrics = Metrics::default();
        let mut colonnes = column_layouts(&schema_exemple(), &metrics);
        let avant = colonnes[1].width;
        fit_columns(
            &mut colonnes,
            &lot_exemple(),
            &FormatOptions::default(),
            &metrics,
            WIDTH_SAMPLE_ROWS,
        );
        assert!(colonnes[1].width >= avant);
        for colonne in &colonnes {
            assert!(colonne.width >= metrics.min_column_width);
            assert!(colonne.width <= metrics.max_column_width);
        }
    }

    #[test]
    fn la_mesure_ne_retrecit_jamais_sous_lentete() {
        let metrics = Metrics::default();
        let mut colonnes = column_layouts(&schema_exemple(), &metrics);
        let entete = colonnes[1].width;
        // Un lot ne contenant que des valeurs courtes ne doit pas ramener la
        // colonne sous la largeur de son nom.
        fit_columns(
            &mut colonnes,
            &lot_exemple(),
            &FormatOptions::default(),
            &metrics,
            1,
        );
        assert!(colonnes[1].width >= entete);
    }

    #[test]
    fn les_noms_de_types_courts_restent_lisibles() {
        assert_eq!(short_type_name(&DataType::Utf8), "text");
        assert_eq!(short_type_name(&DataType::LargeUtf8), "text");
        assert_eq!(
            short_type_name(&DataType::Timestamp(
                arrow::datatypes::TimeUnit::Microsecond,
                None
            )),
            "timestamp"
        );
        assert_eq!(
            short_type_name(&DataType::Timestamp(
                arrow::datatypes::TimeUnit::Microsecond,
                Some("UTC".into())
            )),
            "timestamptz"
        );
        assert_eq!(
            short_type_name(&DataType::Decimal128(12, 2)),
            "decimal(12,2)"
        );
    }

    #[test]
    fn un_lot_deborde_sur_disque_ne_bloque_pas_le_rendu() {
        // `row_batch` est le garde-fou d'I-05 : sur un lot non résident, elle
        // rend `None` au lieu de déclencher une lecture disque.
        let tampon = ResultBuffer::new(schema_exemple(), 1024 * 1024);
        assert!(row_batch(&tampon, 0).is_none(), "tampon vide");
        tampon
            .push(lot_exemple())
            .expect("le lot respecte le schéma du tampon");
        let (_, decalage) = row_batch(&tampon, 2).expect("le lot est en mémoire");
        assert_eq!(decalage, 2);
    }

    #[test]
    fn letat_en_cours_couvre_un_tampon_non_clos() {
        let tampon = Arc::new(ResultBuffer::new(schema_exemple(), 1024));
        let etat = GridState::Streaming(Arc::clone(&tampon));
        assert!(
            etat.is_running(),
            "un tampon non clos est une exécution en cours"
        );
        tampon.mark_complete(oxyn_core::ExecStats::default());
        let etat = GridState::Streaming(tampon);
        assert!(!etat.is_running());
    }

    #[test]
    fn letat_initial_et_letat_erreur_ne_sont_pas_en_cours() {
        assert!(!GridState::Idle.is_running());
        assert!(
            !GridState::Failed {
                message: SharedString::new_static("42P01: relation inexistante"),
                retryable: false,
            }
            .is_running()
        );
        assert!(GridState::Starting.is_running());
    }
}

#[cfg(test)]
mod scroll_tests {
    use super::*;
    use gpui::{ScrollDelta, TestAppContext, TouchPhase, point};

    struct GridHost(gpui::Entity<DataGrid>);

    impl Render for GridHost {
        fn render(&mut self, _: &mut Window, _: &mut Context<'_, Self>) -> impl IntoElement {
            div()
                .size_full()
                .flex()
                .flex_col()
                .child(div().h(px(220.)).flex_none())
                .child(
                    div()
                        .flex_1()
                        .min_h_0()
                        .overflow_hidden()
                        .child(self.0.clone()),
                )
                .child(div().h(px(28.)).flex_none())
        }
    }

    #[gpui::test]
    fn wheel_scrolls_rows_within_the_viewport(cx: &mut TestAppContext) {
        let schema = Arc::new(arrow::datatypes::Schema::new(vec![
            arrow::datatypes::Field::new("id", DataType::Int32, false),
        ]));
        let buffer = Arc::new(ResultBuffer::new(schema.clone(), 1024 * 1024));
        buffer
            .push(
                RecordBatch::try_new(
                    schema,
                    vec![Arc::new(arrow::array::Int32Array::from_iter_values(
                        0..1000,
                    ))],
                )
                .expect("valid batch"),
            )
            .expect("resident batch");
        buffer.mark_complete(oxyn_core::ExecStats::default());
        let (host, cx) = cx.add_window_view(|_, cx| {
            GridHost(cx.new(|cx| {
                let mut grid = DataGrid::new(cx);
                grid.set_buffer(buffer, cx);
                grid
            }))
        });
        let grid = host.read_with(cx, |host, _| host.0.clone());
        cx.run_until_parked();
        let handle = grid.read_with(cx, |grid, _| grid.scroll.clone());
        assert!(handle.is_scrollable(), "rows must exceed the viewport");
        let before = handle.0.borrow().base_handle.offset().y;
        cx.simulate_event(ScrollWheelEvent {
            position: point(px(100.), px(350.)),
            delta: ScrollDelta::Pixels(point(px(0.), px(-300.))),
            touch_phase: TouchPhase::Moved,
            modifiers: Default::default(),
        });
        let after = handle.0.borrow().base_handle.offset().y;
        assert!(
            after < before,
            "wheel must move rows: {before:?} -> {after:?}"
        );
        cx.simulate_event(ScrollWheelEvent {
            position: point(px(100.), px(350.)),
            delta: ScrollDelta::Pixels(point(px(0.), px(10000.))),
            touch_phase: TouchPhase::Moved,
            modifiers: Default::default(),
        });
        cx.run_until_parked();
        assert_eq!(
            handle.0.borrow().base_handle.offset().y,
            Pixels::ZERO,
            "scrolling above the first row must clamp at the top"
        );
        cx.simulate_resize(gpui::size(px(800.), px(600.)));
        cx.run_until_parked();
        assert!(handle.is_scrollable(), "resizing must preserve scrolling");
    }
}
