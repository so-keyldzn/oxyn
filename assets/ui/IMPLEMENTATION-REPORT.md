# Fondations GPUI — compte rendu du lot

Date : 2026-09-07. Périmètre : `theme.rs`, nouveau `icons.rs`, `assets/ui/`
et `assets/fonts/`. Aucun commit ni push.

> **Mise à jour du 2026-09-07, lot suivant — ce document n'est plus à jour sur
> trois points.** Il est conservé tel quel parce qu'il porte les contrôles
> réellement exécutés par son auteur ; ce qui a changé depuis :
>
> 1. **L'intégration décrite plus bas est faite.** `lib.rs` déclare `pub mod
>    icons;` et exporte `IconName, UiAssets, icon, logo` ; `main.rs` appelle
>    `Application::with_assets(UiAssets)` et `text_system().add_fonts(...)`.
>    La phrase « `lib.rs` ne déclare pas encore le module » ne vaut plus.
> 2. **La palette n'est pas « zinc »** mais graphite/os : c'est `README.md` de
>    ce même répertoire, § « Actualisation de la palette », qui est exact.
> 3. **Geist et Hugeicons sont désormais dans `docs/RESEARCH-NOTES.md`**, avec
>    leurs commits amont et la question de redistribution, comme ce document le
>    demandait.
>
> Reste vrai, et non traité : aucune icône n'est encore affichée par une vue, et
> aucun rendu de l'application n'a été inspecté visuellement.

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
