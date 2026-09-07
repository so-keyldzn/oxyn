//! Jetons de couleur, de typographie et de métrique.
//!
//! # Pourquoi un thème plutôt que des couleurs à l'appel
//!
//! Une couleur écrite dans un composant est une couleur qu'on ne peut plus
//! changer : elle n'existe qu'à cet endroit, et la variante claire du produit
//! devient une recherche-remplacement sur trente fichiers. Toute la crate lit
//! [`Theme`] ; un test de `lib.rs` relit les sources des composants et échoue si
//! l'un d'eux construit une couleur au lieu de la demander ici.
//!
//! # Comment il est atteint
//!
//! Le thème est un [`Global`] GPUI, installé une fois par [`Theme::init`].
//! [`Theme::of`] ne panique jamais : si l'installation a été oubliée, elle rend
//! le thème sombre par défaut. Un écran mal coloré est un défaut visible en une
//! seconde ; une panique au premier rendu perd le travail non enregistré de
//! l'utilisateur ([I-09](../../../CLAUDE.md#i-09)).
//!
//! # Ce qui n'est pas ici
//!
//! Les couleurs de coloration syntaxique : l'éditeur ne colore pas encore
//! ([`crate::query_editor`]), et un jeu de jetons sans consommateur serait du
//! code mort.

use std::sync::OnceLock;

use gpui::{App, Global, Hsla, Pixels, SharedString, px, rgb};
use oxyn_core::Environment;

/// Variante d'apparence.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum ThemeMode {
    /// Fond sombre. C'est le défaut : le public d'Oxyn travaille sur des
    /// terminaux et des éditeurs sombres, et l'alternance est fatigante.
    #[default]
    Dark,
    /// Fond clair.
    Light,
}

impl ThemeMode {
    /// Nom stable, celui qui est écrit dans les préférences.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Dark => "dark",
            Self::Light => "light",
        }
    }
}

/// Les couleurs d'une variante.
///
/// Un champ par **rôle**, jamais par teinte : `danger` et non `red`. Une
/// palette nommée par teinte se retrouve à porter du rouge qui ne signale rien
/// et du vert qui alarme.
#[derive(Debug, Clone, Copy, PartialEq)]
#[non_exhaustive]
pub struct Palette {
    /// Fond de la fenêtre.
    pub background: Hsla,
    /// Fond d'un panneau posé sur le fond.
    pub surface: Hsla,
    /// Fond d'un élément surélevé : en-tête de grille, barre d'état.
    pub surface_raised: Hsla,
    /// Fond d'un voile modal.
    pub scrim: Hsla,

    /// Trait de séparation ordinaire.
    pub border: Hsla,
    /// Trait de séparation d'un élément qui a le focus clavier.
    pub border_focus: Hsla,

    /// Texte principal.
    pub text: Hsla,
    /// Texte secondaire : en-têtes, légendes, types de colonnes.
    pub text_muted: Hsla,
    /// Texte tertiaire : numéros de ligne, indications de vide.
    pub text_faint: Hsla,
    /// Texte posé sur un aplat d'accentuation ou de danger.
    pub text_on_accent: Hsla,

    /// Couleur d'accentuation : sélection, curseur, action principale.
    pub accent: Hsla,
    /// Fond d'une ligne ou d'un nœud sélectionné.
    pub selection: Hsla,
    /// Fond d'un élément survolé.
    pub hover: Hsla,

    /// Confirmation, connexion établie.
    pub success: Hsla,
    /// Avertissement : résultat tronqué, schéma déduit.
    pub warning: Hsla,
    /// Danger : erreur, production, opération destructrice.
    pub danger: Hsla,

    /// Fond de la ligne d'en-têtes de la grille.
    pub grid_header: Hsla,
    /// Fond de la gouttière de numéros de ligne.
    pub grid_gutter: Hsla,
    /// Fond d'une ligne sur deux.
    pub grid_stripe: Hsla,
    /// Trait entre deux cellules.
    pub grid_line: Hsla,
    /// Texte d'une cellule absente.
    ///
    /// **Ce champ vaut aujourd'hui la même chose que
    /// [`text_muted`](Self::text_muted) et [`text_faint`](Self::text_faint)**
    /// dans les deux variantes : l'actualisation de palette du 2026-09-07 les a
    /// fait converger vers le jeton `text_muted` de la maquette, faute d'un
    /// jeton dédié. La distinction visuelle d'une valeur absente ne tient donc
    /// plus qu'à l'italique que la grille applique.
    ///
    /// Le champ reste séparé parce que la distinction est **voulue** — une
    /// valeur absente n'est pas une valeur discrète — et qu'un rôle fusionné
    /// avec un autre ne se défusionne plus. À relire dans la maquette :
    /// [RESEARCH-NOTES](../../../docs/RESEARCH-NOTES.md#ce-qui-na-pas-pu-être-lu-et-pourquoi).
    pub null: Hsla,
}

impl Palette {
    /// La palette sombre.
    #[must_use]
    pub fn dark() -> Self {
        Self {
            background: rgb(0x151413).into(),
            surface: rgb(0x1c1b1a).into(),
            surface_raised: rgb(0x262422).into(),
            scrim: hsla_from(rgb(0x151413).into(), 0.72),

            border: rgb(0x443c37).into(),
            border_focus: rgb(0xbf4c22).into(),

            text: rgb(0xeeebea).into(),
            text_muted: rgb(0xb9ada5).into(),
            text_faint: rgb(0xb9ada5).into(),
            text_on_accent: rgb(0xffffff).into(),

            accent: rgb(0xbf4c22).into(),
            selection: rgb(0x262422).into(),
            hover: rgb(0x262422).into(),

            success: rgb(0x4ec9a5).into(),
            warning: rgb(0xe0b155).into(),
            danger: rgb(0xf2616b).into(),

            grid_header: rgb(0x262422).into(),
            grid_gutter: rgb(0x1c1b1a).into(),
            grid_stripe: hsla_from(rgb(0xeeebea).into(), 0.02),
            grid_line: rgb(0x443c37).into(),
            null: rgb(0xb9ada5).into(),
        }
    }

    /// La palette claire.
    #[must_use]
    pub fn light() -> Self {
        Self {
            background: rgb(0xeeebea).into(),
            surface: rgb(0xf7f5f3).into(),
            surface_raised: rgb(0xe3deda).into(),
            scrim: hsla_from(rgb(0x1c1b1a).into(), 0.40),

            border: rgb(0xd5ccc6).into(),
            border_focus: rgb(0xbf4c22).into(),

            text: rgb(0x1c1b1a).into(),
            text_muted: rgb(0x6e625c).into(),
            text_faint: rgb(0x6e625c).into(),
            text_on_accent: rgb(0xffffff).into(),

            accent: rgb(0xbf4c22).into(),
            selection: rgb(0xe3deda).into(),
            hover: rgb(0xe3deda).into(),

            success: rgb(0x1a7f5a).into(),
            warning: rgb(0x9a6b00).into(),
            danger: rgb(0xc0323c).into(),

            grid_header: rgb(0xe3deda).into(),
            grid_gutter: rgb(0xf7f5f3).into(),
            grid_stripe: hsla_from(rgb(0x1c1b1a).into(), 0.025),
            grid_line: rgb(0xd5ccc6).into(),
            null: rgb(0x6e625c).into(),
        }
    }

    /// La couleur qui signale un environnement.
    ///
    /// La production est en [`danger`](Self::danger) **en permanence**, pas
    /// seulement au moment d'écrire : c'est ce que demande
    /// [I-02](../../../CLAUDE.md#i-02). Un badge qui n'apparaît qu'à la
    /// confirmation arrive trop tard — l'utilisateur a déjà tapé sa requête.
    #[must_use]
    pub fn environment(&self, environment: Environment) -> Hsla {
        match environment {
            Environment::Local => self.text_faint,
            Environment::Development => self.success,
            Environment::Staging => self.warning,
            Environment::Production => self.danger,
        }
    }
}

/// Les familles et tailles de caractères.
#[derive(Debug, Clone, PartialEq)]
#[non_exhaustive]
pub struct Typography {
    /// Famille de l'interface.
    pub ui_family: SharedString,
    /// Famille à chasse fixe : éditeur, cellules, aperçus d'instruction.
    ///
    /// Une valeur de base de données s'aligne en colonne ou ne se compare pas.
    pub mono_family: SharedString,
    /// Taille du texte d'interface.
    pub ui_size: Pixels,
    /// Taille du texte à chasse fixe.
    pub mono_size: Pixels,
    /// Taille des mentions secondaires : type d'une colonne, badge.
    pub small_size: Pixels,
    /// Hauteur de ligne de l'éditeur.
    pub line_height: Pixels,
}

impl Default for Typography {
    fn default() -> Self {
        Self {
            // UiAssets::fonts supplies the Figma family at application startup.
            // GPUI retains its platform fallback if font registration fails.
            ui_family: SharedString::new_static("Geist"),
            mono_family: SharedString::new_static("Menlo"),
            ui_size: px(13.0),
            mono_size: px(12.0),
            small_size: px(11.0),
            line_height: px(18.0),
        }
    }
}

/// L'échelle d'espacement de la maquette.
///
/// # Pourquoi une échelle et non des nombres à l'appel
///
/// Un `px(10.)` écrit dans une vue n'est comparable à rien : il ne se relit pas,
/// ne se retrouve pas, et rien ne signale qu'il est le seul de la crate à valoir
/// 10. Une échelle fermée rend l'écart visible — un espacement qui n'a pas de
/// nom ici est un espacement que la maquette ne prévoit pas.
///
/// # D'où viennent les chiffres
///
/// Collection `Oxyn / Primitives` du fichier Figma `Yviemi4brBczzdRdBp1ONv`,
/// variables `space/0`, `space/4`, `space/8`, `space/12`, `space/16` et
/// `space/24`, lues le **2026-09-07** sur les écrans `13:291` (workspace sombre)
/// et `47:8222` (préférences). Ce sont les six seules valeurs que ces deux
/// écrans emploient ; voir [`Radii`] pour la même remarque sur les rayons.
#[derive(Debug, Clone, Copy, PartialEq)]
#[non_exhaustive]
pub struct Spacing {
    /// `space/0` — collé, sans gouttière.
    pub none: Pixels,
    /// `space/4` — entre une icône et son libellé.
    pub tiny: Pixels,
    /// `space/8` — entre deux contrôles d'une même rangée.
    pub small: Pixels,
    /// `space/12` — marge intérieure d'un contrôle, gouttière entre deux champs.
    pub medium: Pixels,
    /// `space/16` — marge d'un panneau.
    pub large: Pixels,
    /// `space/24` — marge d'un écran, entre deux groupes de réglages.
    pub huge: Pixels,
}

impl Default for Spacing {
    fn default() -> Self {
        Self {
            none: px(0.0),
            tiny: px(4.0),
            small: px(8.0),
            medium: px(12.0),
            large: px(16.0),
            huge: px(24.0),
        }
    }
}

/// Les rayons de coin de la maquette.
///
/// Lus le **2026-09-07** dans la collection `Oxyn / Primitives` du fichier
/// Figma `Yviemi4brBczzdRdBp1ONv` : `radius/6`, `radius/8` et `radius/full`.
/// `radius/full` vaut littéralement `999` dans la maquette — c'est la
/// convention qui rend un côté parfaitement semi-circulaire quelle que soit la
/// hauteur ; la valeur est reprise telle quelle plutôt que traduite en une
/// moitié de hauteur qui divergerait au premier changement de contrôle.
#[derive(Debug, Clone, Copy, PartialEq)]
#[non_exhaustive]
pub struct Radii {
    /// `radius/6` — contrôles : bouton, champ, cellule d'en-tête.
    pub control: Pixels,
    /// `radius/8` — surfaces : panneau, boîte de dialogue, carte.
    pub surface: Pixels,
    /// `radius/full` — pastilles et badges d'environnement.
    pub full: Pixels,
}

impl Default for Radii {
    fn default() -> Self {
        Self {
            control: px(6.0),
            surface: px(8.0),
            full: px(999.0),
        }
    }
}

/// Les dimensions qui gouvernent la grille, les listes et le cadre de fenêtre.
///
/// [`row_height`](Self::row_height) est **fixe et non négociable** : c'est la
/// précondition de `uniform_list`, qui mesure un élément et en déduit la
/// position de tous les autres ([ADR-0002](../../../docs/adr/0002-arrow-result-model.md)).
/// Une hauteur de ligne variable rendrait la virtualisation impossible, donc
/// l'affichage de 10 millions de lignes aussi.
///
/// # L'état de la vérification, au 2026-09-07
///
/// Les hauteurs de cadre — barre d'outils, barre d'état, panneau latéral,
/// contrôle — sont **nommées ici parce qu'elles étaient des tailles de frame
/// dispersées**. Chaque champ dit d'où vient son chiffre. Trois d'entre eux
/// viennent du fichier Figma ; deux n'ont pas pu y être relus et le disent.
///
/// Les dimensions de grille, elles, préexistent à cette lecture et **n'ont pas
/// été confrontées à la maquette** : le quota du serveur Figma Dev Mode a été
/// épuisé avant que les pages `03 · Foundations` et `22 · Database workspace`
/// aient pu être localisées. Elles sont conservées telles quelles ; les
/// remplacer par des valeurs plausibles aurait été pire que de les laisser
/// signalées ([I-12](../../../CLAUDE.md#i-12)).
#[derive(Debug, Clone, Copy, PartialEq)]
#[non_exhaustive]
pub struct Metrics {
    /// Hauteur de la barre d'outils du workspace.
    ///
    /// Figma `47:8422` « Workspace toolbar », lu le 2026-09-07.
    pub toolbar_height: Pixels,
    /// Hauteur de la barre d'état.
    ///
    /// **Non relu dans la maquette** : le nœud n'a pas pu être atteint le
    /// 2026-09-07 (quota du serveur Dev Mode épuisé). La valeur vient de la
    /// consigne de tâche et reste à confirmer. Elle corrige tout de même un
    /// défaut réel — [`crate::status_bar`] se dimensionnait jusqu'ici avec
    /// [`header_height`](Self::header_height), c'est-à-dire avec la hauteur
    /// d'en-tête de la grille de résultats.
    pub status_bar_height: Pixels,
    /// Largeur du panneau latéral déplié.
    ///
    /// Figma `8:4` « Sidebar / Expanded » et son instance `13:292`, lus le
    /// 2026-09-07.
    pub sidebar_width: Pixels,
    /// Largeur du panneau latéral replié sur ses icônes.
    ///
    /// Figma `13:165` « Sidebar / Collapsed » et son instance `13:484`, lus le
    /// 2026-09-07.
    pub sidebar_collapsed_width: Pixels,
    /// Hauteur d'un contrôle : bouton, champ de saisie, onglet.
    ///
    /// Figma `47:8461`, `47:8465` et `47:8469` — les trois cadres `Input` de
    /// l'écran de préférences `47:8222` — lus le 2026-09-07.
    pub control_height: Pixels,
    /// Épaisseur de l'anneau de focus clavier.
    ///
    /// **Non relu dans la maquette.** Deux pixels sont le minimum pour qu'un
    /// anneau reste visible sur un écran non HiDPI ; un anneau d'un pixel se
    /// confond avec la bordure ordinaire du contrôle, ce qui revient à ne pas
    /// avoir de focus visible ([ADR-0001](../../../docs/adr/0001-ui-toolkit.md)).
    pub focus_ring: Pixels,
    /// Hauteur d'une ligne de grille ou d'un nœud d'arbre.
    pub row_height: Pixels,
    /// Hauteur de la ligne d'en-têtes.
    pub header_height: Pixels,
    /// Largeur de la gouttière de numéros de ligne.
    pub gutter_width: Pixels,
    /// Largeur minimale d'une colonne, y compris après redimensionnement.
    pub min_column_width: Pixels,
    /// Largeur maximale attribuée par la mesure automatique.
    ///
    /// Une colonne `text` contenant des documents JSON mesurerait sinon dix
    /// mille pixels et pousserait toutes les autres hors de l'écran.
    pub max_column_width: Pixels,
    /// Largeur d'une colonne dont rien ne permet d'estimer le contenu.
    pub default_column_width: Pixels,
    /// Marge horizontale intérieure d'une cellule.
    pub cell_padding: Pixels,
    /// Largeur estimée d'un caractère à chasse fixe.
    ///
    /// Sert **uniquement** au dimensionnement initial des colonnes, jamais au
    /// placement d'un curseur : une estimation qui décale un curseur est un
    /// défaut visible à chaque frappe, alors qu'une colonne de dix pixels trop
    /// large ne se remarque pas et se redimensionne.
    pub char_width: Pixels,
    /// Retrait par niveau dans l'arbre de catalogue.
    pub indent: Pixels,
}

impl Default for Metrics {
    fn default() -> Self {
        Self {
            toolbar_height: px(52.0),
            status_bar_height: px(32.0),
            sidebar_width: px(280.0),
            sidebar_collapsed_width: px(64.0),
            control_height: px(38.0),
            focus_ring: px(2.0),
            row_height: px(24.0),
            header_height: px(28.0),
            gutter_width: px(60.0),
            min_column_width: px(48.0),
            max_column_width: px(480.0),
            default_column_width: px(120.0),
            cell_padding: px(8.0),
            char_width: px(7.2),
            indent: px(14.0),
        }
    }
}

/// Le thème complet, injecté en global GPUI.
#[derive(Debug, Clone, PartialEq)]
#[non_exhaustive]
pub struct Theme {
    /// Variante active.
    pub mode: ThemeMode,
    /// Couleurs.
    pub colors: Palette,
    /// Caractères.
    pub typography: Typography,
    /// Dimensions.
    pub metrics: Metrics,
    /// Espacements.
    pub spacing: Spacing,
    /// Rayons de coin.
    pub radii: Radii,
}

impl Theme {
    /// Le thème sombre.
    #[must_use]
    pub fn dark() -> Self {
        Self {
            mode: ThemeMode::Dark,
            colors: Palette::dark(),
            typography: Typography::default(),
            metrics: Metrics::default(),
            spacing: Spacing::default(),
            radii: Radii::default(),
        }
    }

    /// Le thème clair.
    #[must_use]
    pub fn light() -> Self {
        Self {
            mode: ThemeMode::Light,
            colors: Palette::light(),
            typography: Typography::default(),
            metrics: Metrics::default(),
            spacing: Spacing::default(),
            radii: Radii::default(),
        }
    }

    /// Le thème d'une variante.
    #[must_use]
    pub fn of_mode(mode: ThemeMode) -> Self {
        match mode {
            ThemeMode::Dark => Self::dark(),
            ThemeMode::Light => Self::light(),
        }
    }

    /// Installe le thème comme global de l'application.
    ///
    /// À appeler une fois, au démarrage, avant d'ouvrir la fenêtre. Rappelée
    /// plus tard, elle remplace le thème et les vues se redessinent au cycle
    /// suivant.
    pub fn init(mode: ThemeMode, cx: &mut App) {
        cx.set_global(Self::of_mode(mode));
    }

    /// Le thème courant.
    ///
    /// Ne panique pas si [`init`](Self::init) n'a pas été appelée : rend le
    /// thème sombre. Le rendu ne doit jamais échouer sur un défaut de câblage
    /// du démarrage.
    #[must_use]
    pub fn of(cx: &App) -> &Self {
        match cx.try_global::<Self>() {
            Some(theme) => theme,
            None => fallback(),
        }
    }
}

impl Default for Theme {
    fn default() -> Self {
        Self::dark()
    }
}

impl Global for Theme {}

/// Le thème rendu quand le global n'est pas installé.
///
/// Construit une fois : [`Theme::of`] rend une référence, il lui faut donc une
/// valeur qui vit aussi longtemps que le programme.
fn fallback() -> &'static Theme {
    static FALLBACK: OnceLock<Theme> = OnceLock::new();
    FALLBACK.get_or_init(Theme::dark)
}

/// La même couleur, à une autre opacité.
///
/// GPUI n'expose pas de constructeur direct pour cela ; passer par les champs
/// de [`Hsla`] est la voie courte, et elle est ici plutôt que répétée dans
/// chaque palette.
fn hsla_from(base: Hsla, alpha: f32) -> Hsla {
    Hsla { a: alpha, ..base }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn la_production_est_la_couleur_de_danger_dans_les_deux_variantes() {
        // I-02 : le marquage de la production ne dépend pas de l'apparence
        // choisie. Une variante claire qui l'oublierait rendrait l'avertissement
        // invisible pour la moitié des utilisateurs.
        for palette in [Palette::dark(), Palette::light()] {
            assert_eq!(palette.environment(Environment::Production), palette.danger);
        }
    }

    #[test]
    fn les_quatre_environnements_ont_des_couleurs_distinctes() {
        for palette in [Palette::dark(), Palette::light()] {
            let couleurs = [
                palette.environment(Environment::Local),
                palette.environment(Environment::Development),
                palette.environment(Environment::Staging),
                palette.environment(Environment::Production),
            ];
            for (i, gauche) in couleurs.iter().enumerate() {
                for droite in couleurs.iter().skip(i + 1) {
                    assert_ne!(gauche, droite, "deux environnements se confondent");
                }
            }
        }
    }

    #[test]
    fn le_theme_de_repli_existe_sans_global() {
        // `Theme::of` est appelée à chaque trame ; elle ne doit dépendre
        // d'aucune initialisation pour rendre une valeur.
        assert_eq!(fallback().mode, ThemeMode::Dark);
    }

    #[test]
    fn les_valeurs_absentes_ne_se_confondent_pas_avec_le_texte_principal() {
        // Le nom dit exactement ce qui est vérifié. La comparaison avec
        // `text_faint` **échouerait** aujourd'hui : les deux rôles ont convergé
        // lors de l'actualisation de palette, ce que documente le champ `null`.
        // Écrire ici un test qui passe en laissant croire à une distinction qui
        // n'existe pas serait pire que de ne rien tester.
        for palette in [Palette::dark(), Palette::light()] {
            assert_ne!(palette.null, palette.text);
        }
    }

    #[test]
    fn les_metriques_lues_dans_la_maquette_gardent_leur_valeur() {
        // Ces quatre chiffres viennent du fichier Figma `Yviemi4brBczzdRdBp1ONv`
        // et sont cités nœud par nœud dans la documentation de `Metrics`. Le
        // test existe parce qu'une métrique de maquette se « corrige » très
        // facilement à l'œil pendant un ajustement d'écran : le jour où l'un de
        // ces nombres change, il doit être relu dans la maquette et sa
        // documentation mise à jour, pas ajusté en silence.
        let metrics = Metrics::default();
        assert_eq!(metrics.toolbar_height, px(52.0), "Figma 47:8422");
        assert_eq!(metrics.sidebar_width, px(280.0), "Figma 8:4");
        assert_eq!(metrics.sidebar_collapsed_width, px(64.0), "Figma 13:165");
        assert_eq!(metrics.control_height, px(38.0), "Figma 47:8461");
    }

    #[test]
    fn lechelle_despacement_est_celle_de_la_maquette() {
        // L'échelle est fermée : `space/0` à `space/24`. Un espacement absent
        // d'ici est un espacement que la maquette ne prévoit pas, et l'ajouter
        // demande de le lire d'abord.
        let spacing = Spacing::default();
        assert_eq!(
            [
                spacing.none,
                spacing.tiny,
                spacing.small,
                spacing.medium,
                spacing.large,
                spacing.huge,
            ],
            [px(0.0), px(4.0), px(8.0), px(12.0), px(16.0), px(24.0)],
        );
    }

    #[test]
    fn lanneau_de_focus_est_plus_epais_quune_bordure_ordinaire() {
        // Un anneau d'un pixel se confond avec la bordure du contrôle : le
        // focus serait « présent » sans être visible, ce que la liste de
        // contrôle d'interface refuse comme un défaut bloquant.
        assert!(Metrics::default().focus_ring > px(1.0));
    }

    #[test]
    fn le_panneau_replie_est_plus_etroit_que_le_panneau_deplie() {
        let metrics = Metrics::default();
        assert!(metrics.sidebar_collapsed_width < metrics.sidebar_width);
    }

    #[test]
    fn la_largeur_minimale_de_colonne_est_sous_la_largeur_par_defaut() {
        let metrics = Metrics::default();
        assert!(metrics.min_column_width < metrics.default_column_width);
        assert!(metrics.default_column_width < metrics.max_column_width);
    }
}
