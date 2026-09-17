# ADR-0029 — Interface web dans Tauri : TanStack Start, shadcn/ui sur Base UI

**Statut :** proposé · **Date :** 2026-09-15
**Remplace :** [ADR-0001](0001-ui-toolkit.md) sur le choix du toolkit, et
[ADR-0009](0009-source-dependance-gpui.md) à la suppression des crates GPUI. La
règle d'isolation de l'ADR-0001 reste en vigueur, transposée à Tauri.

## Contexte

L'ADR-0001 écartait Tauri au nom de « native first ». Le coût dominant qu'il
nommait — un éditeur de code et une grille de résultats de qualité
professionnelle — s'est vérifié : `oxyn-ui` et `oxyn-app` totalisent
**~14 500 lignes** le 2026-09-15, dont 1 686 pour la seule grille et 1 245 pour
l'éditeur, sans atteindre le niveau de finition de la maquette Figma. Chaque
composant (champ, menu, dialogue, arbre, infobulle, focus clavier) y est écrit à
la main, et GPUI n'a pas de bibliothèque de composants ni de documentation
([ADR-0001](0001-ui-toolkit.md#conséquences)).

L'écosystème web fournit ces composants tout faits, accessibles, et testables
isolément. Les faits, vérifiés au registre le 2026-09-15
([RESEARCH-NOTES](../RESEARCH-NOTES.md#interface-tauri-et-front)) :

* `tauri 2.11.5` sur crates.io, `rust-version = 1.77.2`, compatible avec la
  toolchain épinglée `1.98.1` ;
* Tauri ne sert que des fichiers statiques : SSG, SPA ou MPA, pas de SSR ;
* `@tanstack/react-start 1.168.54` a un mode SPA qui produit un shell statique ;
* `shadcn 4.21.0` génère un projet TanStack Start sur Base UI
  (`shadcn create -t start -b base`) ;
* `@storybook/addon-vitest 10.6.0` exige `vitest ^3 || ^4` : la série 5
  (5.0.0 publiée le 2026-09-03) n'est pas utilisable avec Storybook.

Le reste du dépôt ne connaît pas l'interface : les onze autres crates
(~45 000 lignes) ne dépendent pas de `gpui` ([I-08](../../CLAUDE.md#i-08)). La
bascule ne touche que les deux crates que l'ADR-0001 avait isolées pour ça.

## Décision

**L'interface d'Oxyn est une application web servie par Tauri 2.**

| Couche | Choix | Où |
|---|---|---|
| Hôte natif | `tauri` 2, fenêtre et IPC | crate `crates/oxyn-desktop`, binaire `oxyn-desktop` |
| Framework front | TanStack Start en **mode SPA** : routage par fichiers, loaders | `apps/desktop` |
| Données côté front | TanStack Query (appels IPC), Table et Virtual (grille), Form, Store, Hotkeys, Pacer | `apps/desktop` |
| Composants | shadcn/ui, style `base-nova`, primitives **Base UI** | `apps/desktop/src/components/ui` |
| Style | Tailwind CSS 4, jetons en variables CSS | `apps/desktop/src/styles.css` |
| Atelier de composants | Storybook 10 avec `addon-vitest` et `addon-a11y` | `apps/desktop/.storybook` |
| Gestionnaire de paquets | `pnpm`, version épinglée par `packageManager` | `apps/desktop/package.json` |

**TanStack Start ne tourne jamais en serveur.** Ni SSR, ni *server functions*, ni
*server routes* : dans Tauri il n'y a pas de serveur HTTP, et le seul backend
est Rust. Tout ce qu'une *server function* ferait passe par une commande Tauri.

**Une commande Tauri ne fait qu'émettre une `Command`.** `crates/oxyn-desktop`
reçoit un appel IPC, le traduit en `oxyn_core::Command` portant `Actor::Human`,
et le confie à l'`Executor`. Aucune commande Tauri n'appelle un driver, le
trousseau ou le store directement : ce serait le second chemin d'exécution que
[I-01](../../CLAUDE.md#i-01) interdit, cette fois exposé à du JavaScript.

**Les résultats traversent l'IPC par pages de cellules formatées.** Le front
demande une fenêtre bornée de lignes (`result_page`, 2 000 lignes au plus) ;
`crates/oxyn-desktop` la lit dans le `ResultBuffer` et formate chaque cellule
avec `oxyn_data::format_cell`, qui repose sur les formateurs d'`arrow-rs`
**utilisés aussi par l'export** et par la grille GPUI. Le front ne
décode pas Arrow et ne reformate rien : un horodatage ou un binaire rendu en
JavaScript divergerait du fichier exporté, ce que
[UX-SPEC](../UX-SPEC.md#ce-qui-est-exporté-est-ce-qui-est-affiché) interdit. Il
ne tient jamais le résultat entier : la grille virtualisée ne garde que les
pages visibles ([I-06](../../CLAUDE.md#i-06), [ADR-0002](0002-arrow-result-model.md)).

**Règle d'isolation, transposée.** Aucune crate hors de `oxyn-desktop` ne dépend
de `tauri`. Jusqu'à la suppression des crates GPUI, l'ancienne règle vaut
toujours pour `gpui`.

**La migration se fait par parité.** `oxyn-ui` et `oxyn-app` restent dans le
workspace et compilent jusqu'à ce que `apps/desktop` couvre les mêmes écrans
([IMPLEMENTATION-PLAN](../IMPLEMENTATION-PLAN.md)). Le binaire `oxyn-desktop`
porte sa propre assemblée du backend : la partager exigerait de sortir
`ConnectionDraft` d'`oxyn-ui`, un travail jeté à la suppression de la crate.

## Conséquences

* **+** Les composants accessibles (clavier, focus, ARIA) viennent de Base UI au
  lieu d'être réécrits ; l'accessibilité cesse d'être le risque structurel que
  nommait l'ADR-0001.
* **+** Chaque composant se développe et se teste isolément dans Storybook, y
  compris ses cinq états ([UX-SPEC](../UX-SPEC.md#états-dune-vue)).
* **+** Windows cesse d'être « fragile » : WebView2 est la cible première de Tauri.
* **+** La maquette Figma se traduit en composants existants, sans moteur de
  rendu à écrire.
* **−** **« Native first — pas de webview, pas de runtime JS » est abandonné.**
  C'est un principe de la [VISION](../VISION.md), corrigé dans le même commit.
* **−** Trois moteurs web : WKWebView (macOS), WebView2 (Windows), WebKitGTK
  (Linux). Un rendu vérifié sur l'un ne l'est pas sur les autres.
* **−** Les budgets de [PERFORMANCE](../PERFORMANCE.md) — trame à 8 ms p99 au
  défilement, démarrage à froid en 1 s — ne sont **plus acquis par
  construction** : ils se remesurent dans la webview. Chaque page de grille
  paie une traversée IPC et une sérialisation JSON de cellules formatées.
* **−** Nouvelle surface d'entrée : le contenu d'une cellule, un nom d'objet du
  catalogue ou une réponse de modèle rendus dans le DOM. Une XSS dans la webview
  atteint les commandes Tauri. D'où : CSP stricte, *capabilities* Tauri
  minimales, aucun `dangerouslySetInnerHTML` sur une donnée reçue
  ([SECURITY](../SECURITY.md#surface-dentrée)).
* **−** Deux chaînes d'outils : `cargo` et `pnpm`. `make qualite` doit couvrir
  les deux, sinon le front échappe à la porte.
* **−** Pendant la migration, deux interfaces coexistent et le backend est
  assemblé deux fois.

**Coût de sortie :** réécrire `apps/desktop` et `crates/oxyn-desktop`. Le
reste du produit ne connaît ni Tauri ni React. Ce coût croît avec le nombre
d'écrans ; il est borné par la règle d'isolation, qui doit tenir.

**Reconsidérer si** la grille ne tient pas **8 ms p99** au défilement d'un
résultat d'un million de lignes sur macOS et Linux, mesuré dans la webview de
production ; ou si WebKitGTK rend l'application inutilisable sous Linux.

## Alternatives écartées

| Alternative | Raison du rejet |
|---|---|
| Rester sur GPUI | le coût de chaque composant écrit à la main dépasse celui de la webview, et l'écart avec la maquette ne se résorbe pas |
| egui | même problème que GPUI, avec un éditeur à construire en plus ([ADR-0001](0001-ui-toolkit.md)) |
| Electron | embarque Chromium et Node dans chaque paquet, et un runtime JS côté backend alors que le backend est Rust |
| TanStack Start avec SSR ou *server functions* | Tauri ne sert que des fichiers statiques ; un serveur Node embarqué dédoublerait le backend |
| shadcn/ui sur Radix | Base UI retenu par choix produit ; `shadcn` supporte les deux, et le skill `shadcn` du dépôt indique les écarts d'API (`render` et non `asChild`) |
| Vitest 5 | `@storybook/addon-vitest 10.6.0` n'accepte que `^3 \|\| ^4` |
| Arrow IPC décodé dans le front (`apache-arrow`) | le formatage d'une cellule se ferait deux fois, en Rust pour l'export et en JavaScript pour l'écran, et les deux divergeraient ; une page de 2 000 lignes formatées reste petite |
| TypeScript 7 | le squelette généré par `shadcn 4.21.0` fixe `typescript ^6` ; monter en 7 est une décision séparée, à vérifier contre ESLint et le docgen de Storybook |
