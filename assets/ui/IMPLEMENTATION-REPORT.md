# Fondations GPUI — compte rendu du lot

Date : 2026-09-07. Périmètre : `theme.rs`, nouveau `icons.rs`, `assets/ui/`
et `assets/fonts/`. Aucun commit ni push.

## Livré

- Palette zinc Figma claire/sombre dans l’interface `Theme` existante ; couleurs
  sémantiques de succès, avertissement et danger conservées.
- Douze SVG Hugeicons Stroke Rounded et logo officiel, téléchargés depuis les
  exports Figma exacts, sans modification des octets ni substitution Unicode.
- `IconName::{Database, Table, Search, Plus, Panel, Settings, Book, Terminal,
  History, Folder, Chevron, Down, Logo}` et `icon(IconName) -> gpui::Svg`.
- Boîtes explicites 16 × 16 px ; logo 32 × 32 px, marge interne conservée.
- `UiAssets: gpui::AssetSource` charge des octets statiques empruntés, sans I/O.
- `UiAssets::fonts()` retourne les trois TTF Geist incluses à la compilation.
- Notices, nœuds Figma et SHA-256 dans les fichiers de provenance adjacents.

## Intégration à effectuer par le coordinateur

Dans `lib.rs` :

```rust
pub mod icons;
pub use icons::{IconName, UiAssets, icon};
```

Au démarrage : `Application::new().with_assets(UiAssets)` et, avant les fenêtres,
`cx.text_system().add_fonts(UiAssets::fonts())`. Signaler toute erreur
d’enregistrement ; le moteur de texte garde son repli de plateforme.
Chaque appelant fournit la teinte, par exemple
`icon(IconName::Search).text_color(theme.colors.text_muted)`.

Ajouter les sources datées de Geist et Hugeicons à `docs/RESEARCH-NOTES.md`
dans le lot d’intégration : ce document est hors du périmètre de ce worker.
Références précises dans les deux `provenance.json` et README.

## Contrôles exécutés

- `rustfmt --edition 2024` puis `--check` sur les deux fichiers Rust : succès.
- `cargo check -p oxyn-ui --locked` : succès. Deux avertissements de compatibilité
  future amont (`block`, `proc-macro-error2`), aucun échec de compilation.
- `icons.rs` compilé séparément avec `rustc --emit=metadata`, lié aux métadonnées
  GPUI produites par Cargo : succès. Nécessaire car `lib.rs` ne déclare pas encore
  le module dans ce worktree.
- `make socle` : 41/41 cas conformes ; avertissement existant de règle `tests.md`
  sans chemin correspondant.
- `git diff --check` sur le périmètre : succès.
- Treize SVG : XML valide, SHA-256 conforme à la provenance, dimensions et
  `viewBox` vérifiés, absence de scripts, images ou liens externes, rasterisation
  `rsvg-convert` réussie. Recherche 16 px et logo 32 px inspectés visuellement.
- Trois polices : structure SFNT, bornes des tables, famille typographique Geist
  et SHA-256 vérifiés. Les noms de famille hérités de Medium/SemiBold incluent
  leur graisse ; le nom de famille typographique commun est bien Geist.
- Relecture locale des invariants : aucune I/O de rendu, aucune interaction
  distante, aucune modification de politique ou de stockage ; rien de bloquant
  dans le périmètre.

## Limites

Le coordinateur doit exécuter `cargo fmt --all` et `make qualite` sur l’ensemble
intégré, puis vérifier le rendu dans l’application. Ce lot ne revendique ni
validation des interactions, ni capture GPUI, ni mesure des budgets de trame.
La notice Hugeicons autorise l’utilisation telle quelle et restreint la
redistribution du jeu : les modalités de publication du dépôt restent à
vérifier, aucune licence MIT n’étant déduite. La licence Geist SIL OFL est jointe.
