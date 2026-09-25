# ADR-0041 — Un registre d'actions unique alimente menus, raccourcis et palette

**Statut :** proposé · **Date :** 2026-09-25

**Précise :** [ADR-0029](0029-interface-tauri-shadcn.md), sur deux points : comment
un déclencheur autre qu'un bouton atteint le pont IPC, et ce que la fenêtre
native fait quand Oxyn ne dit rien. Le tableau de l'ADR-0029 citait TanStack
Hotkeys parmi les briques du front : il cesse d'être la voie des raccourcis
d'application (point 3).

**Précise :** [ADR-0038](0038-un-plantage-s-annonce-une-fois.md), sur la forme
de son menu macOS : la copie du menu par défaut de Tauri dont seul Quit change
(`application_menu`, `crates/oxyn-desktop/src/commands/recovery.rs`) devient la
barre construite depuis le manifeste (point 4). Le Quit d'Oxyn, son raccourci
et son traitement en Rust par l'arrêt ordonné ne changent pas.

## Contexte

Oxyn doit offrir les interactions d'une application de bureau — barre de menus,
clic droit, raccourcis, palette — à un public qui vient de DBeaver, SSMS ou
DataGrip. **Ce qu'elles font** est fixé par
[UX-SPEC](../UX-SPEC.md), section « Menus, raccourcis et gestes » et ses
sous-sections « Barre de menus », « Menus contextuels », « Clavier », « Souris
et glisser », « Ce qu'Oxyn ne fait pas, parce que ce n'est pas un navigateur ».
Cet ADR fixe **comment** elles sont construites, et ce qu'elles ne doivent
jamais devenir : un second chemin d'exécution ([I-01](../../CLAUDE.md#i-01)).

### Ce qui existe le 2026-09-25

Les raccourcis sont déclarés par **quatre mécanismes, dans 27 fichiers**, sans
lieu où un conflit se verrait :

| Mécanisme | Où |
|---|---|
| `useHotkeys` / `useHotkey` (`@tanstack/react-hotkeys`, qui se déclare alpha — [RESEARCH-NOTES](../RESEARCH-NOTES.md)) | `features/workspace/workspace-screen.tsx` (9 liaisons : `⌘T`, `⌘W`, `⌘J`, `⌃Tab`, `⌃⇧Tab`, `⌘1`, `⌘2`, `⌘⇧H`, `⌘⌥B`), `features/recovery/recovery-screen.tsx` (`⌘[`), `features/settings/settings-dialog.tsx` (`⌘,`) |
| `window.addEventListener("keydown")` | `features/connections/connection-screen-view.tsx`, `components/oxyn/connection-form.tsx`, `components/oxyn/saved-connections.tsx`, et `components/ui/sidebar.tsx` (généré par shadcn : `⌘B` par `event.key === "b"`) |
| keymap CodeMirror | `components/oxyn/sql-editor.tsx` (`Mod-Enter`, `Mod-Shift-Enter`, `Mod-s`, `Escape` en `Prec.highest`), plus les liaisons par défaut de `basicSetup` |
| `onKeyDown` de composant | 19 composants de `components/oxyn/`, dont `result-grid.tsx`, `catalog-tree.tsx`, `workspace-tabs.tsx` — la plupart pour le motif ARIA de leur widget — et `components/ui/carousel.tsx`, généré |

Relevé le 2026-09-25 sur `origin/main` (`b86b4b9`).

Chemins relatifs à `apps/desktop/src/`. Deux défauts se lisent déjà dans ce
code : `result-grid.tsx` teste `event.metaKey || event.ctrlKey`, donc `⌃C`
copie aussi sous macOS ; `sidebar.tsx` compare `event.key` à une lettre latine,
ce qui échoue sur une disposition cyrillique. Et deux raccourcis d'Oxyn
tombent sur des liaisons par défaut de CodeMirror : `@codemirror/commands`
6.11.0 (installé en dépendance transitive de `@uiw/react-codemirror`) lie
`Mod-/` à `toggleComment`, et `@codemirror/search` lie `Mod-f` à son panneau
de recherche. Enfin, sous macOS, `⌘W` est lié deux fois : par
`workspace-screen.tsx`, qui ferme l'onglet actif, et par l'élément prédéfini
`Close Window` que le menu d'ADR-0038 garde dans `File` et dans `Window`
(accélérateur `⌘W` de `muda`, ci-dessous). Si c'est le menu qui reçoit la
frappe (point à vérifier n° 1), `performClose:` produit un `CloseRequested`,
que `on_run_event` transforme en arrêt ordonné : `⌘W` quitte Oxyn.

Un seul menu contextuel existe, celui de l'arbre du catalogue
(`components/oxyn/catalog-tree.tsx`, shadcn `context-menu` sur Base UI), qui
copie un nom **cité par le backend** (`qualified_name` de `RelationFacets`,
`crates/oxyn-desktop/src/ipc/metadata.rs`). `menubar` est installé dans
`components/ui/` et utilisé nulle part ; `command` ne sert qu'au choix du
driver (`components/oxyn/driver-choices.tsx`) ; `kbd` affiche déjà des
raccourcis dans quinze fichiers, chacun écrit à la main à côté de la liaison
qu'il décrit.

### Ce que la fenêtre native fait par défaut

Vérifié le 2026-09-25 dans le source des versions de `Cargo.lock` : `tauri`
2.11.5, `tauri-utils` 2.9.3, `tauri-runtime-wry` 2.11.4, `wry` 0.55.1, `muda`
0.19.3 ([I-12](../../CLAUDE.md#i-12)).

1. **Sous macOS, Tauri installe un menu quand on ne lui en donne pas.**
   `Builder::build` (`src/app.rs`) pose `Menu::default` si aucun menu n'est
   fourni et que `enable_macos_default_menu` vaut `true`, ce qui est le défaut.
   Ce menu contient un menu d'application (About, Services, Hide, Hide Others,
   Quit), `File` (Close Window), `Edit` (Undo, Redo, Cut, Copy, Paste, Select
   All), `View` (Fullscreen), `Window` (Minimize, Maximize, Close Window) et
   `Help` (`src/menu/menu.rs`). Sous Windows et Linux, aucun menu n'est posé.
   Depuis [ADR-0038](0038-un-plantage-s-annonce-une-fois.md),
   `crates/oxyn-desktop/src/main.rs` fournit sous macOS
   `commands::recovery::application_menu` : `Menu::default`, dont le menu
   d'application est reconstruit à l'identique sauf son dernier élément, un
   `MenuItem` d'identifiant `oxyn-quit` (`CmdOrCtrl+Q`) que `on_run_event`
   traite en lançant l'arrêt ordonné. Tout le reste — `Close Window` compris —
   est celui de Tauri.
2. **Les entrées `Edit` sont des rôles système.** `muda` les lie aux sélecteurs
   AppKit `copy:`, `paste:`, `cut:`, `selectAll:`, `undo:`, `redo:`, envoyés
   dans la chaîne de répondeurs ; `Quit` prédéfini est `terminate:`,
   `Close Window` est `performClose:` (`src/platform_impl/macos/mod.rs`).
   Leurs accélérateurs par défaut sont dans `src/items/predefined.rs` : `⌘W`
   pour `Close Window` sous macOS (`Alt+F4` ailleurs), `⌘Q` pour `Quit`, `⌘H`
   pour `Hide`, `⌘⌥H` pour `Hide Others`. `terminate:` n'est pas retenable :
   tao ne traite pas `applicationShouldTerminate:`, et la sortie arrive en
   `RunEvent::Exit` sans `ExitRequested` — c'est ce qui a fait écrire
   [ADR-0038](0038-un-plantage-s-annonce-une-fois.md) pour ⌘Q et
   [ADR-0040](0040-inscrire-la-fermeture-d-une-sortie-forcee.md) pour le Quit
   du Dock et la fermeture de session ([RESEARCH-NOTES](../RESEARCH-NOTES.md#interface-tauri-et-front)).
3. **Un élément de menu n'a pas d'infobulle, et son raccourci est une
   liaison.** `muda` 0.19.3 expose `set_text`, `set_enabled` et
   `set_accelerator` sur un élément ; aucune occurrence de *tooltip* dans la
   crate. Un raccourci affiché est un raccourci capté : il n'existe pas
   d'affichage seul.
4. **Recevoir un événement de Rust par `listen` exige une permission.**
   `tauri` 2.11.5 range `event` parmi ses permissions de cœur, comme `menu`.
   `capabilities/main.json` n'accorde que trois permissions, et sa description
   nomme `event` et `menu` parmi ce que l'ensemble par défaut ajouterait. Les
   commandes d'Oxyn et les `Channel` n'y sont pas soumis (même description) :
   c'est par eux que passent déjà les flux (`subscribe_events`,
   `commands/subscriptions.rs`).
5. **`titleBarStyle` ne vaut que pour macOS** (« The style of the macOS title
   bar », `tauri-utils` `config.rs`). Sous Windows et Linux, la fenêtre garde
   la barre de titre du système ; l'en-tête d'Oxyn est la première rangée de la
   page, sous elle.
6. **Le zoom de page est coupé par défaut, pas le reste.** `zoomHotkeysEnabled`
   vaut `false` : sous Windows il pilote `IsZoomControlEnabled` de WebView2,
   sous macOS et Linux aucun polyfill n'est injecté. `wry` met
   `back_forward_navigation_gestures` à `false` et `tauri-runtime-wry` ne le
   change pas. En revanche `with_browser_accelerator_keys` de `wry` (vrai par
   défaut : `Ctrl+R`, `F5`, `Ctrl+P`, `Ctrl+F` de WebView2) n'est pas exposé par
   `tauri-runtime-wry` 2.11.4. `allowLinkPreview` vaut `true` par défaut sous
   macOS.
7. **Le dépôt de fichiers du système exclut le glisser HTML5 sous Windows.**
   `dragDropEnabled` vaut `true` par défaut, et sa documentation dit :
   « Disabling it is required to use HTML5 drag and drop on the frontend on
   Windows ».

## Décision

### 1. Un registre d'actions, en deux parties

**Toute action d'interface a une identité unique**, déclarée dans
`apps/desktop/src/lib/actions/` :

- **le manifeste**, `actions.json` — la partie statique : `id`, libellé (et ses
  variantes par zone, voir 2), zone, raccourci par plateforme (`mac`,
  `other`), place dans la barre de menus (menu, groupe, ordre, mnémonique),
  indicateur `destructive`. Rien d'exécutable ;
- **les comportements**, `registry.ts` — pour chaque `id`, `enabled(context)`,
  qui rend `true`, une raison, ou l'absence (les cas d'absence sont ceux
  qu'UX-SPEC fixe déjà), et `run(context)`.

```ts
type ActionBehaviour = {
  enabled: (context: ActionContext) => true | { reason: string } | "absent"
  run: (context: ActionContext) => void | Promise<void>
}
```

`ActionContext` porte la zone de focus, l'onglet actif, la sélection, la cible
d'un menu contextuel, et la connexion active (environnement, capacités,
lecture seule). Il est recalculé sur changement de focus, d'onglet, de
sélection ou d'état d'exécution — pas à chaque frappe — et comparé au
précédent : rien ne part vers les déclencheurs s'il n'a pas changé.

L'identifiant est un espace de noms stable, `<domaine>.<verbe>` (`console.run`,
`tab.close`, `app.quit`). Le préfixe `plugin.` est réservé : une action de
plugin ([ADR-0005](0005-wasm-plugins.md)) sera **déclarative** — elle nommera
une `Command` —, jamais une fonction du front, parce qu'un composant WASM n'en
fournit pas ([PLUGIN-CONTRACT](../PLUGIN-CONTRACT.md), point 1).

**Les déclencheurs ne font que lire le registre** : barre de menus, menus
contextuels, clavier, palette, feuille des raccourcis — **et les boutons** qui
portent une action du registre, qui l'invoquent par son `id` au lieu de garder
leur propre condition de grisé. C'est ce qui garantit que `Run` est grisé
« dans les mêmes cas que son bouton » (UX-SPEC, « Barre de menus ») : il n'y a
qu'une condition.

**`invoke(id, source)` réévalue `enabled` au moment de l'appel.** Le grisé d'un
menu n'est qu'un affichage : une activation arrivée après un changement de
contexte est ignorée, avec une trace, jamais exécutée sur un état périmé.

**Deux tests tiennent le registre**, dans `make front` : chaque `id` du
manifeste a un comportement et réciproquement ; et les règles de conflit des
points 2 et 3 sont vérifiées sur le manifeste entier. Un conflit de raccourci ne
se voit jamais à l'exécution — l'une des deux actions ne part simplement pas.

### 2. Un raccourci se résout par zone de focus

Les zones sont nommées dans le DOM (`data-action-zone` : `editor`, `grid`,
`tree`, `tabs`, `assistant`…), et la zone active est la plus proche ancêtre de
`document.activeElement`. Un dialogue modal ouvert est une zone qui masque
toutes les autres, comme le fait déjà `workspace-screen.tsx` (« Dialogs own the
keyboard while open »).

- **Un raccourci est unique par zone** : jamais deux actions pour la même
  combinaison dans la même zone.
- **Une zone plus précise l'emporte sur le niveau global.** `⌘/` commente la
  ligne dans `editor` et ouvre la feuille des raccourcis ailleurs ; `⌘F` est une
  seule action, `edit.find`, qui cherche dans la zone qui a le focus.
- **Le menu suit la zone active.** Un élément dont la combinaison est masquée
  par une zone plus précise s'affiche sans elle ; celui qui la porte dans la
  zone active l'affiche ; un libellé qui dépend de la zone prend la variante
  déclarée au manifeste pour elle.
- **Entre les onglets : `⌃Tab` et `⌃⇧Tab` seulement.** Aucun `⌘1…9` :
  `⌘1` montre le catalogue et `⌘2` donne le focus à la grille de l'aperçu
  (`workspace-screen.tsx` ; UX-SPEC, « Clavier »).
- **Les touches internes à un composant ne sont pas des actions.** Flèches,
  `Home`/`End`, `PageUp`/`PageDown`, `Entrée` et `Échap` qui déplacent le focus
  ou referment une surface appartiennent au motif ARIA du composant et restent
  dans son `onKeyDown`. Le critère : une touche qui a une entrée de menu ou de
  palette est une action ; une touche qui navigue dans un widget n'en est pas
  une. La feuille des raccourcis les liste quand même, déclarées au manifeste
  comme appartenant à leur zone.
- **Le keymap CodeMirror ne lie plus une combinaison que le manifeste
  connaît.** `sql-editor.tsx` construit ses liaisons d'Oxyn depuis le registre,
  et retire des liaisons par défaut celles que le manifeste déclare pour la zone
  `editor` (`Mod-/`, `Mod-f`) : elles passent par le registre, qui appelle la
  commande CodeMirror correspondante. Une combinaison liée à deux endroits se
  déclenche deux fois, ou une seule selon l'ordre de livraison — dans les deux
  cas sans erreur.

### 3. Le clavier : un répartiteur, des règles de correspondance écrites

**Un seul écouteur `keydown`**, en phase de capture sur `window`, posé par
`lib/actions/keyboard.ts`. Les usages de `useHotkeys`, les écouteurs `keydown`
de fenêtre et le `⌘B` de `sidebar.tsx` migrent vers lui ; ce dernier, généré par
shadcn et non retouché à la main, est désactivé par son fournisseur ou déclaré
au manifeste comme appartenant au composant, pour que la feuille le montre et
que le test de conflit le voie. Le répartiteur est écrit par Oxyn plutôt que
délégué à `@tanstack/react-hotkeys` : les règles ci-dessous sont les nôtres, et
la bibliothèque se déclare alpha.

- **`Mod` vaut `metaKey` sous macOS et `ctrlKey` ailleurs — jamais l'un ou
  l'autre.**
- **L'identité d'une touche** : `event.key` quand c'est un caractère ASCII
  imprimable unique ; sinon la lettre ou le chiffre dérivé de `event.code`
  (`KeyC` → `c`). Cela couvre `⌥` sous macOS, qui change le caractère produit,
  et les dispositions non latines. **Les chiffres se lisent toujours par
  `event.code`** : en AZERTY, ils exigent `⇧`.
- **`⇧` n'est pas comparé pour une ponctuation**, puisqu'il sert à la produire
  (`/` est `⇧:` en AZERTY). En conséquence, **aucune action ne déclare
  `Mod+Shift+<ponctuation>`** ; le test du manifeste le refuse.
- **Sous Windows et Linux, aucune action ne déclare `Ctrl+Alt+<touche
  imprimable>`** : `AltGr` y est rapporté comme `Ctrl+Alt`, et la combinaison
  vole un caractère (en AZERTY français, `AltGr+0` produit `@`).
- **Une frappe en cours de composition (`event.isComposing`) n'est jamais un
  raccourci** : une méthode de saisie CJK valide par `Entrée`.
- **`⇧F10` et la touche `Menu`** sont l'action `context-menu.open` : elle émet
  `contextmenu` sur l'élément focalisé, que le menu de sa zone traite comme un
  clic droit — c'est ce que fait déjà l'arbre du catalogue sur la ligne
  focalisée.
- **Une action destructrice (`destructive: true`) n'a pas de raccourci.** Un
  raccourci rend réflexe ce que la revue de l'ADR-0042 existe pour ralentir.

### 4. macOS : barre de menus native, décrite par le manifeste

`crates/oxyn-desktop/src/menu.rs` construit la barre avec `tauri::menu` au
démarrage, **depuis le manifeste**, lu par `include_str!` et parsé par `serde`
— un test Rust le parse et construit la description du menu, pour qu'un
manifeste invalide échoue en CI et non au lancement. La barre existe donc avant
que la webview ait chargé, et ses libellés ne viennent jamais de JavaScript.

Elle remplace `application_menu` d'ADR-0038, donc la copie de `Menu::default`,
et en reprend explicitement ce qui relève du système, en éléments prédéfinis de
`muda` : About, Services, Hide, Hide Others, Show All, Fullscreen, Minimize,
Zoom, et **Cut, Copy, Paste, Select All** — les rôles qui atteignent un champ
de texte de la webview (« Contexte », point 2). **Trois rôles ne sont pas
repris en prédéfini** :

- **Quit** reste l'élément d'ADR-0038 : l'action `app.quit` du manifeste, `⌘Q`,
  dont l'élément natif porte l'identifiant que `on_run_event` reconnaît
  (aujourd'hui `oxyn-quit`). **C'est la seule entrée que Rust exécute lui-même**,
  sans aller-retour par la webview : l'arrêt ordonné vide les brouillons de la
  webview, et une webview figée ne doit pas empêcher de quitter. L'élément
  appelle la même fonction que `request_exit` (point 5) : résolution des
  transactions ouvertes (ADR-0043), puis arrêt ordonné. Le prédéfini
  `terminate:` n'est pas retenable (« Contexte », point 2) ;
- **Close Window** prédéfini porte `⌘W`, qu'Oxyn donne à `tab.close` ; la
  fermeture de fenêtre est une action du registre, sans `⌘W`. Ce retrait
  ferme le double lien relevé au « Contexte » ;
- **Undo** et **Redo** : UX-SPEC leur donne une portée qui dépasse le texte
  (la disposition). Ils sont des actions du registre si le point à vérifier
  n° 3 confirme qu'une action peut défaire l'édition d'un champ natif ; sinon
  ils restent prédéfinis, limités au texte, et la disposition a son entrée
  propre.

**Aller.** Un clic, ou un raccourci capté par la barre, arrive en
`RunEvent::MenuEvent`, là où `on_run_event` traite déjà le Quit d'ADR-0038.
Hors `app.quit`, `menu.rs` vérifie que l'`id` appartient au manifeste et
l'envoie à la fenêtre active par un `Channel` ouvert par la commande
`subscribe_menu` (`commands/menu.rs`, même modèle de remplacement que
`commands/subscriptions.rs`). Pas d'`emit` : `listen` exigerait une permission
`core:event` (« Contexte », point 4). Le message ne porte **qu'un `id`**, validé
par un schéma `zod` à l'entrée ([ADR-0031](0031-validation-des-reponses-ipc.md),
point 5), puis `invoke(id, "menu")`. **`menu.rs` ne construit aucune
`Command`** : il ne sait pas ce que fait l'action.

**Retour.** La commande `set_menu_state` reçoit, pour chaque élément du
manifeste présent dans la barre : actif ou non, indice de variante de libellé,
raccourci montré ou non. Elle n'accepte ni texte libre ni combinaison
arbitraire — une XSS ne peut pas renommer « Quit » ni lier une touche —, et un
`id` inconnu est une erreur non rejouable. Elle est synchrone, donc sur le
thread principal : elle n'y fait que des appels `set_enabled`, `set_text`,
`set_accelerator`, sans I/O ([I-05](../../CLAUDE.md#i-05)). Le front ne l'appelle
que sur changement du contexte (point 1).

**Sous macOS, une combinaison a une seule liaison** : si elle est portée par
un élément natif dans la zone active, le répartiteur du point 3 l'ignore, et
`set_menu_state` déplace l'accélérateur quand la zone change (`⌘/` passe de
`Keyboard shortcuts` à `Toggle comment` quand l'éditeur prend le focus). Cette
règle rend la question de l'ordre de livraison (point à vérifier n° 1)
indifférente pour les combinaisons avec `⌘`.

**`Échap` n'est jamais un accélérateur natif.** `Query ▸ Cancel` capterait
`Échap` partout — fermeture d'un dialogue, d'une complétion de l'éditeur, d'un
menu —, et `muda` n'offre pas d'affichage sans liaison (« Contexte »,
point 3). L'entrée native `Cancel` s'affiche donc **sans raccourci** sous macOS ;
`Échap` reste traité par la zone de la console, comme aujourd'hui
(`sql-editor.tsx`, barre de la console), et figure dans la palette et la
feuille des raccourcis. `⌘.` n'est pas utilisé.

La barre native vise la fenêtre active : la répartition entre plusieurs
fenêtres — un `Channel` par fenêtre, l'état poussé par celle qui prend le
focus — relève de l'ADR-0043.

**Le menu du Dock reste celui du système.** Cet ADR n'y ajoute aucune entrée.
Son `Quit`, comme la fermeture de session macOS, envoie `terminate:` : ce
chemin ne passe pas par le registre, n'est pas retenable, et inscrit la
fermeture comme le décide [ADR-0040](0040-inscrire-la-fermeture-d-une-sortie-forcee.md),
brouillons non vidés.

### 5. Windows et Linux : barre de menus web

La barre est le composant shadcn `menubar`, dans la première rangée de la page
— sous la barre de titre du système, que `titleBarStyle` ne supprime pas
(« Contexte », point 5). Elle lit le même manifeste. `Alt` révèle les
mnémoniques, `F10` lui donne le focus ; sous 1200 px, elle se replie en un seul
bouton de menu (UX-SPEC, « Largeur réduite »). Elle affiche aussi les
raccourcis que tient une zone, `Échap` compris : ici, afficher n'est pas lier.

Aucun menu natif n'est posé sous ces plateformes : c'est déjà le comportement
de Tauri sans menu, et `application_menu` n'est construit que sous macOS
(« Contexte », point 1).

**`File ▸ Exit`** — arbitré par l'utilisateur le 2026-09-25 — est l'action
`app.quit` sous Windows et Linux, raccourci `Ctrl+Q`. Elle quitte toutes les
fenêtres d'un geste, par le chemin de `⌘Q`. Elle appelle une commande IPC
dédiée, **`request_exit`** (`commands/recovery.rs`) :

- **sans argument**, et sans valeur de retour autre que l'acquittement ;
- elle n'appelle que la fonction Rust que `on_run_event` appelle pour
  `oxyn-quit` et `CloseRequested` : la résolution des transactions ouvertes
  d'ADR-0043, puis l'arrêt ordonné d'ADR-0021 et ADR-0038. Elle ne choisit
  rien — ni fenêtre, ni délai, ni saut d'étape — et ne peut donc pas
  demander une sortie qui ne vide pas les brouillons ou n'inscrit pas la
  fermeture ;
- elle est idempotente : `begin_shutdown` ne démarre l'arrêt qu'une fois, et
  un appel pendant une résolution de transactions en cours ne fait rien ;
- elle n'émet aucune `Command` : ce n'est pas une action sur une donnée, mais
  de la tuyauterie d'interface, comme `subscribe_shutdown` et
  `shutdown_flushed`.

Elle est enregistrée sur les trois plateformes : sous macOS, la palette
l'appelle pour `app.quit`, la barre native gardant son élément traité en Rust.

**Effet sur la surface d'entrée** ([SECURITY](../SECURITY.md#surface-dentrée),
point 5). Une XSS gagne un moyen de **demander** la sortie ordonnée — ce que
fermer la fenêtre fait déjà —, rien de plus : les brouillons sont vidés, la
fermeture est inscrite, une transaction ouverte ouvre le dialogue d'ADR-0043
au lieu d'être validée ou annulée en silence. Elle ne lit rien, n'écrit rien
sur un serveur et ne franchit aucune confirmation. Le pire est un déni de
service : l'application qui se ferme, et se rouvre avec ses consoles.

Passer la fenêtre sans décoration
(`decorations: false`, contrôles de fenêtre dessinés par Oxyn) n'est **pas**
décidé ici : il faudrait des permissions `core:window:*` supplémentaires.

### 6. Menus contextuels : web, depuis le registre

Chaque surface qu'UX-SPEC énumère (« Menus contextuels ») a un menu shadcn
`context-menu` sur Base UI, dont les entrées sont des actions du registre
filtrées par surface, avec la cible dans le `ActionContext`. Le précédent est
`catalog-tree.tsx`. Chaque surface a ses stories, axe compris : c'est ce qui
rend ce choix vérifiable, et c'est ce que perdrait un menu natif.

### 7. Palette, ouverture rapide, feuille des raccourcis

- **`⌘K`** : la palette, sur shadcn `command` (`cmdk`, déjà dépendance). Elle
  liste les actions du registre et appelle `invoke(id, "palette")`. Elle
  complète la barre de menus, elle ne la remplace pas.
- **`⌘P`** : l'ouverture rapide d'un objet, sur la recherche existante du
  catalogue (`search_catalog`), qui ne couvre que les objets chargés et le dit.
  `Ctrl+P` étant l'impression de WebView2, le répartiteur l'intercepte.
- **`⌘/`** hors de l'éditeur : la feuille des raccourcis, **générée depuis le
  manifeste** avec `kbd`. Aucune liste de raccourcis n'est écrite à la main
  ailleurs, et les `kbd` qui affichent aujourd'hui un raccourci écrit à côté de
  sa liaison (« Contexte ») le lisent au manifeste.

### 8. Ce qui trahirait la webview

Les comportements sont ceux d'UX-SPEC ; les mécanismes sont ceux-ci.

- **Menu contextuel de page** : un écouteur `contextmenu` sur `document` appelle
  `preventDefault`, sauf sur un champ de texte natif (`input`, `textarea`) sans
  menu propre. L'éditeur SQL a le sien.
- **Rechargement** : `⌘R`, `⌘⇧R`, `Ctrl+R`, `F5` sont réservés au manifeste
  comme combinaisons neutralisées ; le répartiteur les absorbe. Un rechargement
  détruit l'état non sauvegardé, et laisse orphelines les tâches qui
  alimentaient les `Channel` (commentaire de tête de
  `commands/subscriptions.rs`).
- **Zoom de page** : `zoomHotkeysEnabled` reste à `false`. Le pincement est
  neutralisé hors du diagramme `erd` (point à vérifier n° 5).
- **Navigation arrière** : les gestes de balayage sont coupés par défaut ;
  les boutons 4 et 5 de la souris sont absorbés (`mouseup`/`auxclick`) ; `⌘[`
  et `Alt+←` ne sont que les actions nommées de retour qu'UX-SPEC prévoit. En
  dernier rempart, `on_navigation` côté Rust refuse toute navigation hors de
  l'origine de l'application, et `on_new_window` toute nouvelle fenêtre.
- **Champs de texte** : `spellcheck="false"`, `autocorrect="off"`,
  `autocapitalize="off"`, `autocomplete="off"`, sur les champs et sur la surface
  de CodeMirror (`contentAttributes`). `components/ui/` n'étant pas retouché à
  la main, les attributs sont posés par les composants d'Oxyn qui les
  utilisent, et un contrôle de `make front` refuse un `Input` ou un `Textarea`
  qui ne les reçoit pas. Le risque visé : les guillemets typographiques de
  macOS dans une chaîne de connexion ou un littéral SQL (point à vérifier n° 4).
- **Sélection** : `user-select: none` sur le cadre, `text` sur les contenus —
  c'est déjà `styles.css` (`body`, puis `input`, `textarea`, `.cm-editor`,
  `[data-selectable]`) ; cet ADR le fixe.
- **Liens externes** : aucun `<a href>` externe n'est rendu dans la webview.
  Oxyn ouvre ses propres liens (documentation) par une commande
  `open_external`, qui n'accepte que `https:` et que des URL qu'Oxyn déclare —
  jamais une URL lue dans une cellule, un commentaire du catalogue ou une
  réponse de modèle, dont l'ouverture exfiltrerait ce que porte sa requête.
  `allowLinkPreview` passe à `false`.

### 9. Ce que shadcn ne fournit pas

**Règle générale : un geste système — choisir un chemin, ouvrir un lien,
révéler un fichier, recevoir un dépôt — s'exécute en Rust, derrière une
commande d'Oxyn qui le borne.** Aucune permission de plugin n'est ajoutée à
`capabilities/main.json` pour eux. `dialog:allow-open`, déjà accordé à l'écran
de connexion, n'est pas rouvert ici.

- **Dialogues de fichier** : `tauri-plugin-dialog`, appelé depuis Rust par la
  commande qui consomme le chemin (exporter, ouvrir un `.sql`). Une destination
  d'écriture ne vient jamais de JavaScript.
- **Dépôt de fichiers du système** : `WindowEvent::DragDrop` traité dans
  `crates/oxyn-desktop`, les chemins classés (`.sql`, fichier de base lisible
  par un driver enregistré, autre) et transmis au front par un `Channel`. Un
  `.sql` est lu en Rust, sur le pool bloquant, avec une borne de taille, et
  ouvert dans une console sans exécution.
- **Glisser-déposer interne** (onglets, colonnes, catalogue vers l'éditeur) :
  **sur les événements de pointeur, pas sur l'API HTML5**, puisque le dépôt de
  fichiers garde `dragDropEnabled` à `true` et que cela exclut le glisser HTML5
  sous Windows (« Contexte », point 7). Le choix d'une bibliothèque ou d'une
  implémentation propre est différé ; une bibliothèque passera par
  [`/versions`](../../.claude/commands/versions.md). Déposer une table dans
  l'éditeur insère le nom **cité par le backend**, à la position du pointeur,
  sans rien exécuter.

### 10. Les invariants, action par action

- **[I-01](../../CLAUDE.md#i-01).** Un menu est un déclencheur de plus. Toute
  entrée aboutit à `invoke`, qui appelle les mêmes fonctions de
  `lib/ipc/` qu'un bouton ; la `Command` naît dans le backend, comme pour tout
  appel IPC. Les éléments natifs prédéfinis n'agissent que sur le système et la
  webview (presse-papiers d'un champ, fenêtre, visibilité de l'application),
  jamais sur une donnée.
- **[I-02](../../CLAUDE.md#i-02).** Une entrée `destructive` — `Drop…`,
  `Truncate…`, `Rename…`, `Delete…` — n'exécute rien : elle ouvre la revue sur
  place de l'ADR-0042, quel que soit le déclencheur. Elle n'a pas de raccourci
  (point 3), et n'existe pas pour un agent. Aucune entrée, aucun raccourci
  n'accorde une approbation sur `production` : elle passe par le dialogue natif
  d'[ADR-0037](0037-dialogue-natif-pour-les-confirmations-critiques.md), que
  seul le backend ouvre.
- **[I-03](../../CLAUDE.md#i-03).** `Copy connection` ne copie que ce que le
  front détient déjà : les paramètres et la référence de secret d'une connexion
  ne traversent jamais l'IPC ([ARCHITECTURE](../ARCHITECTURE.md), « L'interface
  Tauri »), et **aucune commande n'est ajoutée pour produire une chaîne de
  connexion à copier**. Toute copie passe par `copyToClipboard`
  (`features/metadata/clipboard.ts`), que le test sentinelle du canal
  presse-papiers couvre.
- **[I-06](../../CLAUDE.md#i-06).** `Copy values` d'une colonne copie les lignes
  que le résultat détient, lues par pages bornées, dans la limite
  `MAX_COPY_ROWS` (2 000, `components/oxyn/grid-selection.ts`) ; au-delà,
  elle refuse et renvoie à `Export…`. Elle ne lit jamais la suite du curseur et
  ne relance jamais la requête. Le libellé dit le nombre de lignes.
- **[I-10](../../CLAUDE.md#i-10).** Tout SQL qu'une action compose —
  `Copy as ▸ Quoted name`, `SELECT *`, `INSERT template`, `Copy rows as ▸
  INSERT` ou `IN list`, le nom inséré par glisser — est produit **côté Rust**,
  identifiants cités et littéraux échappés par le driver, comme le sont déjà
  `qualified_name` et `related_rows_template`. Le front n'a aucun gabarit de
  chaîne SQL.
- **[I-07](../../CLAUDE.md#i-07).** Aucune action ne prend en entrée le texte
  d'un modèle pour l'exécuter. Un bloc de code de l'assistant offre `Copy code`
  et `Open in console` ; `Open in console` pose du texte, et c'est `Run`, dans
  la console, qui exécute.
- **[I-04](../../CLAUDE.md#i-04).** `Send to assistant` ne transmet de valeurs
  que par l'écran d'approbation d'un échantillon, comme l'écrit UX-SPEC.

## Conséquences

* **+** Une action a une définition, donc une condition de grisé, un libellé
  et un raccourci : le bouton, le menu, la palette et la feuille ne peuvent plus
  diverger.
* **+** Les conflits de raccourcis deviennent un échec de test, au lieu d'une
  action qui ne part pas.
* **+** Aucune permission n'est ajoutée à la webview : ce qu'une XSS peut faire
  des menus se limite à invoquer des actions qu'un clic offre déjà.
* **+** La barre native existe dès le démarrage et ses libellés ne dépendent
  d'aucun JavaScript.
* **−** **Deux barres de menus**, native sous macOS, web ailleurs, depuis un
  manifeste. La native ne se teste pas en story : seuls sa construction et le
  refus d'un `id` inconnu se testent, en Rust.
* **−** **Une crate Rust lit un fichier du front** (`include_str!` vers
  `apps/desktop/src/lib/actions/actions.json`) : un couplage de chemins entre
  les deux chaînes d'outils, qui casse au premier déplacement du fichier.
* **−** **Sous macOS, la raison d'un grisé ne se lit pas dans la barre** :
  `muda` n'a pas d'infobulle. La palette la montre. C'est un écart avec UX-SPEC
  (« grisée, avec sa raison, lisible au survol »), signalé plutôt que tranché.
* **−** **Sous macOS, `Cancel` s'affiche sans `Échap`** dans la barre native.
* **−** Après un changement de zone, l'accélérateur natif est déplacé par un
  appel IPC : une frappe arrivée pendant cette latence peut viser l'action de
  la zone précédente. `invoke` réévalue `enabled`, mais pas l'intention.
* **−** Un répartiteur clavier écrit par Oxyn, et ses tests par disposition,
  à maintenir au lieu d'une bibliothèque.
* **−** Deux commandes IPC de plus (`subscribe_menu`, `set_menu_state`), plus
  `request_exit`, `open_external` et la transmission des dépôts : chacune est une surface à
  relire comme un changement de sécurité ([SECURITY](../SECURITY.md#surface-dentrée),
  point 5). Ces commandes n'émettent pas de `Command` : ce ne sont pas des
  actions, mais de la tuyauterie d'interface, comme `subscribe_events`.
* **−** Le glisser sur événements de pointeur coûte plus de code que l'API
  HTML5, et CodeMirror n'y reçoit plus un dépôt natif : la position
  d'insertion se calcule (`posAtCoords`).
* **−** Les actions de plugin auront besoin d'une section dynamique de la barre
  native ; elle n'est pas conçue ici, seulement rendue possible par l'espace de
  noms réservé et le caractère déclaratif exigé.

**Coût de sortie :** abandonner la barre native pour la barre web sous macOS,
c'est supprimer `menu.rs` et deux commandes — quelques jours —, en rendant à
macOS un menu minimal qui garde le Quit d'ADR-0038 : sans lui, `⌘Q` retombe
sur `terminate:`. Abandonner le
registre, c'est redisperser chaque action dans ses déclencheurs : un coût qui
croît avec le nombre d'actions, borné parce que toutes appellent les mêmes
fonctions de `lib/ipc/` et que le manifeste en tient la liste.

**Reconsidérer si** l'un des points à vérifier ci-dessous contredit ce qui est
décidé ; si tao traite `applicationShouldTerminate:` (le Quit pourrait
redevenir un rôle prédéfini, comme le prévoit ADR-0038) ; si Tauri expose `AreBrowserAcceleratorKeysEnabled` (le point 8 s'en
simplifierait) ; si la fenêtre sans décoration est décidée sous Windows et
Linux (la barre web y rejoindrait la barre de titre) ; si les plugins de
phase 4 contribuent des actions ; ou si des raccourcis personnalisables sont
demandés — le manifeste deviendrait une couche par défaut, et les surcharges un
fichier de workspace lisible ([I-11](../../CLAUDE.md#i-11)).

## Points à vérifier

Non vérifiés à une source le 2026-09-25 ; aucun n'est écrit ailleurs comme un
fait. La question du chemin de `terminate:`, ouverte à la rédaction, est
tranchée par ADR-0038 et ADR-0040 (« Contexte », point 2).

1. **L'ordre de livraison sous WKWebView** d'une frappe `⌘` entre la page et la
   barre native, et si `preventDefault` dans la page empêche l'élément de menu.
   La règle « une seule liaison » du point 4 est conçue pour ne pas en dépendre.
   La réponse compte dès aujourd'hui : elle dit si `⌘W` ferme l'onglet ou
   quitte Oxyn (« Contexte »).
2. **Que, sans les éléments `Edit` prédéfinis, `⌘C` et `⌘V` cessent de marcher
   dans un champ de la webview sous macOS.** Rapporté couramment, non vérifié.
   La décision garde ces éléments de toute façon.
3. **Qu'une action du registre puisse défaire l'édition d'un `input` natif**
   (`document.execCommand("undo")` sous WebKit), et que `undo:` atteigne
   l'historique de CodeMirror. Décide si Undo et Redo sont des actions ou des
   rôles (point 4).
4. **Que `autocorrect="off"` coupe les guillemets et tirets typographiques de
   macOS dans WKWebView**, qui relèvent du réglage système de substitution
   de texte.
5. **Le pincement** : grossissement désactivé par défaut dans WKWebView, et
   comportement de WebKitGTK.
6. **Que `preventDefault` sur `keydown` neutralise les raccourcis de navigateur
   de WebView2** (`Ctrl+R`, `F5`, `Ctrl+P`), puisque Tauri n'expose pas leur
   interrupteur ; et que `⇧F10` n'y émette pas déjà `contextmenu`, ce qui
   ouvrirait le menu deux fois.
7. **L'affichage d'une combinaison sur la disposition active** qu'exige UX-SPEC
   (« Clavier ») : `navigator.keyboard.getLayoutMap()` n'est pas garanti hors
   Chromium, et macOS peut relocaliser lui-même les équivalents clavier de la
   barre native. Tant que ce n'est pas vérifié, une ponctuation s'affiche par
   son caractère (`⌘/`), pas par la touche à presser (`⌘⇧:` en AZERTY).

## Alternatives écartées

| Alternative | Raison du rejet |
|---|---|
| API JavaScript `@tauri-apps/api/menu` | exige des permissions `menu:*`, et `core:event` pour écouter les clics, dans `capabilities/main.json` : une XSS pourrait créer, renommer ou déclencher des éléments de menu. La description de ce fichier écarte `menu` explicitement |
| Menu natif sur les trois plateformes | sous Windows et Linux, il s'empile sous la barre de titre du système, au-dessus de l'en-tête d'Oxyn ; il ne suit pas le thème ; il ne se teste pas en story. Le rendu sous GTK est une appréciation du 2026-09-25, non mesurée |
| Barre web aussi sous macOS | un utilisateur macOS attend la barre globale ; Hide, Services, Window et les rôles `Edit` y vivent, et une barre d'écran vide paraît cassée |
| Palette seule, sans barre de menus | un public venu de DBeaver, SSMS ou DataGrip découvre les fonctions et apprend leurs raccourcis en parcourant les menus ; une palette suppose de connaître le nom de ce qu'on cherche, et n'offre pas de structure navigable au lecteur d'écran |
| Menus contextuels natifs (`popup` de `muda`) | un aller-retour IPC par clic droit, avec la cible à décrire à Rust ; ni story, ni axe, ni thème ; et les entrées contextuelles changent avec chaque cible, là où la barre est stable |
| Menu décrit en Rust, à côté du registre TypeScript | deux déclarations des mêmes libellés et raccourcis, qui divergent ; c'est la règle écrite à deux endroits que le dépôt refuse |
| Menu décrit par le front et envoyé à Rust au chargement | la barre n'existe qu'après le chargement de la webview, et une XSS pourrait en réécrire les libellés — renommer une entrée pour en faire cliquer une autre |
| `@tanstack/react-hotkeys` comme répartiteur | se déclare alpha ; ses règles de correspondance pour AZERTY, `⌥` et les dispositions non latines ne sont pas vérifiées ; la résolution par zone n'est pas son modèle |
| Laisser chaque composant tenir ses raccourcis | c'est l'état actuel : quatre mécanismes, aucun lieu où un conflit se voit, deux défauts déjà présents |
| Glisser HTML5, `dragDropEnabled` à `false` | supprime le dépôt de fichiers du système sous Windows, qu'UX-SPEC exige |
| Historique mémoire pour le routeur | ferme la navigation arrière par construction, mais change le routage de toute l'application pour un défaut que l'interception et `on_navigation` ferment ; reste la réponse si un chemin de retour leur échappe |
| `Échap` comme accélérateur natif de `Cancel` | capte `Échap` dans toute la fenêtre pendant une exécution : un dialogue ou une complétion ne se ferme plus, la requête s'annule à la place |
