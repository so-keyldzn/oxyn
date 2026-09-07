//! Les composants d'interface d'Oxyn, en GPUI.
//!
//! Cette crate **rend des pixels**. Elle lit des types du noyau, du catalogue et
//! des tampons de résultats, et elle n'en contient aucune logique métier : pas
//! de driver, pas de politique, pas d'ordonnanceur, pas d'analyse de SQL.
//!
//! | Module | Sujet | Autorité |
//! |---|---|---|
//! | [`theme`] | couleurs, typographie, dimensions, espacements, rayons | — |
//! | [`icons`] | les glyphes et les polices embarqués | — |
//! | [`controls`] | survol, focus, pression, indisponibilité d'un contrôle | [ADR-0001], revue-ui |
//! | [`data_grid`] | la grille virtualisée sur `ResultBuffer` | [ADR-0002], PERFORMANCE |
//! | [`format_settings`] | les réglages d'affichage des cellules | — |
//! | [`catalog_tree`] | l'arbre paresseux sur `CatalogCache` | ARCHITECTURE §6 |
//! | [`query_editor`] | l'éditeur de requêtes | [ADR-0001] |
//! | [`status_bar`] | connexion, environnement, coût, annulation | [I-02], UX-SPEC |
//! | [`approval`] | la boîte d'approbation du Policy gate | [ADR-0004], [I-02] |
//! | [`connection_form`] | le choix du type de base et ses paramètres | [ADR-0003], [I-03] |
//!
//! # Les trois règles de cette crate
//!
//! **Une vue ne fait rien elle-même.** Chaque composant expose un type
//! d'événement — [`GridEvent`], [`CatalogTreeEvent`], [`EditorEvent`],
//! [`StatusBarEvent`], [`ApprovalEvent`], [`ConnectionFormEvent`],
//! [`FormatSettingsEvent`] — et
//! `oxyn-app` les traduit en
//! [`Command`](oxyn_core::Command). Aucun composant n'appelle un driver, ne
//! construit une requête ni ne décide d'une politique
//! ([I-01](../../CLAUDE.md#i-01)).
//!
//! **Le thread d'interface ne fait pas d'I/O** ([I-05](../../CLAUDE.md#i-05)).
//! Les deux endroits où c'était tentant sont documentés à leur place :
//! [`data_grid::row_batch`] refuse de relire un lot débordé sur disque, et
//! [`CatalogTree::rebuild`] n'attend jamais le verrou du cache.
//!
//! **Aucune couleur hors de [`theme`]**, et rien d'affiché qui ne doive l'être :
//! ni identifiant de connexion, ni valeur liée
//! ([I-03](../../CLAUDE.md#i-03)).
//!
//! # Ce qui est délibérément incomplet en phase 0
//!
//! [`query_editor`] n'a **ni coloration syntaxique, ni complétion, ni curseurs
//! multiples**. C'est le plus gros poste de travail restant du projet
//! ([ARCHITECTURE §12](../../docs/ARCHITECTURE.md)) ; l'en-tête du module dit
//! précisément ce qui manque et ce qui débloque chaque point.
//!
//! # Câblage attendu, côté `oxyn-app`
//!
//! ```ignore
//! use gpui::prelude::*;
//! use oxyn_ui::{DataGrid, GridEvent, Theme, ThemeMode};
//!
//! Theme::init(ThemeMode::Dark, cx);
//!
//! let grille = cx.new(DataGrid::new);
//! cx.subscribe(&grille, |_app, grille, evenement, cx| match evenement {
//!     // La vue demande ; c'est `oxyn-app` qui soumet au command bus.
//!     GridEvent::CancelRequested => { /* Command::Cancel { .. } */ }
//!     GridEvent::RowSelected(_) => {}
//!     _ => {}
//! })
//! .detach();
//! ```
//!
//! [ADR-0001]: ../../docs/adr/0001-ui-toolkit.md
//! [ADR-0003]: ../../docs/adr/0003-driver-capabilities.md
//! [ADR-0002]: ../../docs/adr/0002-arrow-result-model.md
//! [ADR-0004]: ../../docs/adr/0004-command-bus.md
//! [I-02]: ../../CLAUDE.md#i-02
//! [I-03]: ../../CLAUDE.md#i-03

pub mod approval;
pub mod catalog_tree;
pub mod connection_form;
pub mod controls;
pub mod data_grid;
pub mod format_settings;
pub mod icons;
pub mod query_editor;
pub mod status_bar;
mod text_field;
pub mod theme;

pub use approval::{
    ApprovalDialog, ApprovalEvent, ApprovalId, ApprovalOutcome, ApprovalRequest, actor_label,
};
pub use catalog_tree::{
    CatalogTree, CatalogTreeEvent, FILTER_RESULT_LIMIT, TreeRow, filtered_rows, flatten,
    icon_for_kind, icon_for_node,
};
pub use connection_form::{
    ConnectionDraft, ConnectionForm, ConnectionFormEvent, DriverChoice, FormField, FormFieldKind,
    FormModel, FormState, SavedConnection, environment_choice_label,
};
pub use controls::{ControlState, ControlTone, control, focus_ring};
pub use data_grid::{
    ColumnLayout, DataGrid, GridEvent, GridState, INFERRED_FIELD_KEY, VisibleColumns,
    WIDTH_SAMPLE_ROWS, column_layouts, fit_columns, row_batch, visible_columns,
};
pub use format_settings::{FormatSettings, FormatSettingsEvent, apercu};
pub use icons::{IconName, UiAssets, icon, logo};
pub use query_editor::{
    EditorEvent, LinePiece, QueryEditor, TAB_WIDTH, TextBuffer, TextPosition, UNDO_DEPTH,
    line_pieces,
};
pub use status_bar::{
    ActiveConnection, ExecutionStatus, StatusBar, StatusBarEvent, environment_label,
    format_duration, format_stats,
};
pub use theme::{Metrics, Palette, Radii, Spacing, Theme, ThemeMode, Typography};

#[cfg(test)]
mod tests {
    /// Les sources des composants, telles qu'elles sont compilées.
    ///
    /// `include_str!` et non une lecture de fichier : le chemin est résolu à la
    /// compilation, donc le test ne dépend pas du répertoire courant et ne peut
    /// pas rater un fichier déplacé — il ne compilerait plus.
    const COMPOSANTS: [(&str, &str); 10] = [
        ("approval.rs", include_str!("approval.rs")),
        ("catalog_tree.rs", include_str!("catalog_tree.rs")),
        ("connection_form.rs", include_str!("connection_form.rs")),
        ("controls.rs", include_str!("controls.rs")),
        ("data_grid.rs", include_str!("data_grid.rs")),
        ("format_settings.rs", include_str!("format_settings.rs")),
        ("icons.rs", include_str!("icons.rs")),
        ("query_editor.rs", include_str!("query_editor.rs")),
        ("status_bar.rs", include_str!("status_bar.rs")),
        ("text_field.rs", include_str!("text_field.rs")),
    ];

    #[test]
    fn aucune_couleur_nest_construite_hors_du_theme() {
        // Une couleur écrite dans un composant n'existe qu'à cet endroit : la
        // variante claire devient alors une recherche-remplacement sur toute la
        // crate, et le premier oubli passe inaperçu jusqu'à ce qu'un
        // utilisateur voie du texte noir sur fond noir.
        for (nom, source) in COMPOSANTS {
            for constructeur in ["rgb(", "rgba(", "hsla(", "Hsla {"] {
                assert!(
                    !source.contains(constructeur),
                    "{nom} construit une couleur ({constructeur}) : elle doit venir de `theme`"
                );
            }
        }
    }

    #[test]
    fn aucun_composant_ne_construit_de_commande() {
        // I-01 : une vue émet un événement, elle ne fabrique pas de `Command` et
        // n'appelle aucun driver. Le jour où un composant en construit une
        // « juste pour ce cas », le second chemin d'exécution est créé et il ne
        // disparaît plus.
        //
        // Fil de détente et non preuve : le test lit le code hors commentaires,
        // il ne l'analyse pas. Ce qu'il attrape, c'est l'ajout distrait — qui
        // est le mode de panne réel.
        for (nom, source) in COMPOSANTS {
            for (numero, ligne) in source.lines().enumerate() {
                let code = ligne.split("//").next().unwrap_or_default();
                assert!(
                    !code.contains("Command"),
                    "{nom}:{} touche au command bus : c'est le travail d'`oxyn-app`",
                    numero.saturating_add(1)
                );
            }
        }
    }
}

/// Les types dont une vue de `oxyn-app` a besoin en pratique.
pub mod prelude {
    pub use crate::approval::{ApprovalDialog, ApprovalEvent, ApprovalOutcome, ApprovalRequest};
    pub use crate::catalog_tree::{CatalogTree, CatalogTreeEvent};
    pub use crate::connection_form::{ConnectionDraft, ConnectionForm, ConnectionFormEvent};
    pub use crate::controls::{ControlState, ControlTone, control};
    pub use crate::data_grid::{DataGrid, GridEvent, GridState};
    pub use crate::format_settings::{FormatSettings, FormatSettingsEvent};
    pub use crate::icons::{IconName, UiAssets, icon, logo};
    pub use crate::query_editor::{EditorEvent, QueryEditor};
    pub use crate::status_bar::{ActiveConnection, ExecutionStatus, StatusBar, StatusBarEvent};
    pub use crate::theme::{Theme, ThemeMode};
}
