# Notes de vérification

> **Autorité** : les versions et faits externes cités ailleurs dans le dépôt.
> Toute valeur ci-dessous porte sa source et sa date. Une valeur non datée est
> une valeur périmée qu'on n'a pas encore repérée.

Invariant lié : [I-12](../CLAUDE.md#i-12) — aucune version ni limite externe
recopiée de mémoire.

## Comment re-vérifier

```bash
.claude/hooks/verifier_versions.py
```

Le script interroge crates.io et le canal stable de Rust, compare avec les
valeurs de ce fichier et signale les écarts. Il ne modifie rien : c'est à un
humain de décider d'une montée de version. La commande `/versions` fait la
même chose en expliquant les écarts.

## Chaîne d'outils Rust

| Fait | Valeur | Source | Vérifié le |
|---|---|---|---|
| Rust stable courante | `1.98.1` (48a229cea, 2026-09-01) | `https://static.rust-lang.org/dist/channel-rust-stable.toml` | 2026-09-05 |
| Toolchain épinglée par le dépôt | `1.98.1` | `rust-toolchain.toml` | 2026-09-05 |
| Édition exigée par `gpui` | `2024` | crates.io API, `gpui@0.2.2` | 2026-09-05 |
| Rust minimum pour l'édition 2024 | `1.85` | Édition 2024 stabilisée dans Rust 1.85 | 2026-09-05 |

### MSRV imposés par les dépendances

Relevés dans `~/.cargo/registry/src/.../<crate>/Cargo.toml`, champ
`rust-version`. C'est le plancher réel du workspace : `cargo` refuse de
construire un paquet dont le `rust-version` dépasse la toolchain.

| Crate | `rust-version` | Vérifié le |
|---|---|---|
| `wasmtime` `48.0.1` | **`1.95.0`** | 2026-09-05 |
| `sqlx` `0.9.0`, `sqlx-core`, `sqlx-postgres` | `1.94.0` | 2026-09-05 |
| `arrow` `59.3.0` | `1.85` | 2026-09-05 |
| `rusqlite` `0.37.0` | aucun | 2026-09-05 |
| `gpui` `0.2.2` | aucun | 2026-09-05 |

> **Le plancher est `1.95.0`, imposé par `wasmtime`.** Il ne se voit pas à la
> construction par défaut : `wasmtime` est derrière la fonctionnalité
> `wasm-host` d'`oxyn-plugin`, désactivée. Seul `sqlx` (`1.94.0`) fait échouer
> `cargo check` aujourd'hui. Le jour où quelqu'un active `wasm-host`, c'est
> `1.95.0` qu'il faut — d'où le `rust-version` du workspace fixé à `1.95`, et
> non à `1.94` que la seule erreur observée suggérerait.

> **Écart résolu le 2026-09-05.** La machine de développement était en `1.89.0`,
> neuf versions mineures derrière la stable, et `rust-toolchain.toml` épinglait
> cette valeur — celle que l'ADR-0008 écarte explicitement. `cargo check` a
> tranché : `sqlx 0.9.0` exige `1.94.0`. La toolchain est passée à `1.98.1`,
> conformément à la recommandation de
> [ADR-0008](adr/0008-chaine-outils-rust.md).

## Interface Tauri et front

Relevé le 2026-09-15 au registre npm (`npm view <paquet> version`) et sur
crates.io (API `/api/v1/crates/<crate>`), pour l'[ADR-0029](adr/0029-interface-tauri-shadcn.md).
Les versions sont écrites **exactes** dans `apps/desktop/package.json` et le
`Cargo.toml` racine ; `pnpm-lock.yaml` fige le reste du graphe.

### Crates

| Crate | Version | `rust-version` | Vérifié le |
|---|---|---|---|
| `tauri` | `2.11.5` | `1.77.2` | 2026-09-15 |
| `tauri-build` | `2.6.3` | `1.77.2` | 2026-09-15 |
| `tauri-plugin-dialog` | `2.7.3` | `1.77.2` | 2026-09-15 |
| `rfd` | `0.16.0` | — (non déclaré) | 2026-09-24 |

`rfd` montre le dialogue d'échec de démarrage, avant que l'application Tauri
n'existe. La version est celle que `tauri-plugin-dialog@2.7.3` résout déjà
(`Cargo.lock`, exigence `0.16`) : la dernière publiée est `0.17.2` (crates.io,
2026-09-24), mais la prendre mettrait deux `rfd` dans le graphe. Déclarée sans
fonctionnalités par défaut : celles du plugin (`gtk3`, `common-controls-v6`)
s'unifient sur la même crate.

Comportements du dialogue de message sur lesquels repose
[ADR-0037](adr/0037-dialogue-natif-pour-les-confirmations-critiques.md),
vérifiés dans la source le 2026-09-25 — **à relire à chaque montée du plugin
ou de `rfd`** :

- `tauri-plugin-dialog` 2.7.3, `src/lib.rs`, `MessageDialogBuilder::show` : le
  rappel reçoit `true` pour `Ok`, `Yes`, ou un bouton personnalisé dont le
  libellé **égale** celui de confirmation ; `false` pour toute autre issue,
  fermeture comprise. Deux libellés égaux feraient d'Annuler une confirmation ;
- même crate, `src/desktop.rs`, `show_message_dialog` : le dialogue s'ouvre
  par `run_on_main_thread`, dont l'échec est ignoré ; le rappel n'est alors
  jamais appelé, et un canal abandonné doit valoir refus ;
- `rfd` 0.16.0 (dépendance du plugin), `src/backend/macos/message_dialog.rs` :
  les boutons sont ajoutés à l'`NSAlert` dans l'ordre, confirmation en premier ;
  le premier bouton d'un `NSAlert` répond à Entrée, et le plugin n'offre aucun
  moyen d'en désigner un autre.

### Paquets npm

| Paquet | Version retenue | Dernière publiée | Pourquoi l'écart |
|---|---|---|---|
| `pnpm` (`packageManager`) | `11.1.2` | `12.4.2` | version installée sur la machine de dev ; monter est un commit délibéré |
| `@tauri-apps/cli` · `@tauri-apps/api` | `2.11.4` · `2.11.1` | idem | — |
| `@tanstack/react-start` · `react-router` · `router-plugin` | `1.168.54` · `1.170.36` · `1.168.38` | idem | — |
| `@tanstack/react-query` · `react-virtual` · `react-form` | `5.102.8` · `3.14.13` · `1.33.5` | idem | — |
| `@tanstack/react-store` · `react-pacer` | `0.11.1` · `0.23.0` | idem | `react-hotkeys` (`0.10.0`, qui se déclarait **alpha** dans son README) est retiré le 2026-09-25 : le répartiteur du registre d'actions le remplace ([ADR-0041](adr/0041-registre-d-actions-menus-et-raccourcis.md), point 3) |
| `@tanstack/react-table` | `8.21.3` | `9.2.4` | les composants et exemples shadcn supposent l'API v8 |
| `shadcn` (CLI) · `@base-ui/react` | `4.21.0` · `1.8.0` | idem | style `base-nova` |
| `@hugeicons/react` · `@hugeicons/core-free-icons` | `1.1.10` · `4.3.3` | idem | UX-SPEC impose Hugeicons |
| `react` · `react-dom` | `19.3.0` | idem | — |
| `vite` · `@vitejs/plugin-react` | `8.3.0` · `6.1.1` | idem | — |
| `tailwindcss` · `@tailwindcss/vite` | `4.3.3` | idem | — |
| `typescript` | `6.0.3` | `7.0.2` | voir ADR-0029, alternatives écartées |
| `storybook` · `@storybook/react-vite` · addons | `10.6.0` | idem | — |
| `vitest` · `@vitest/browser-playwright` | `4.1.11` | `5.0.1` | `@storybook/addon-vitest@10.6.0` exige `vitest ^3 \|\| ^4` |
| `playwright` | `1.63.0` | idem | Chromium seul, pour les stories |
| `@uiw/react-codemirror` · `@codemirror/lang-sql` | `4.25.11` · `6.10.0` | idem | — |
| `zod` | `4.6.5` | idem | relevé le 2026-09-16 ; valide les réponses IPC ([ADR-0031](adr/0031-validation-des-reponses-ipc.md)). Était déclaré depuis ADR-0029 et importé nulle part |
| `lexical` · `@lexical/react` | `0.51.0` | idem | relevé le 2026-09-24 (`npm view`, dist-tag `latest`, publié le 2026-09-23) ; le champ de question de l'assistant et ses mentions `@`. `@lexical/react` déclare `yjs` en pair : il ne sert qu'au plugin de collaboration, non importé |

| `@xyflow/react` | `12.11.6` | idem | relevé le 2026-09-24 (publiée le 2026-09-01) ; le diagramme des tables. Pairs `react >=17` |
| `@dagrejs/dagre` | `3.1.1` | idem | relevé le 2026-09-24 (publiée le 2026-08-08) ; disposition du diagramme. **Pas** `dagre` 0.8, abandonné, ni `elkjs` |
| `shiki` | `4.4.3` | idem | relevé le 2026-09-24 (publiée le 2026-08-10) ; coloration du code des réponses, en **moteur JavaScript** (`shiki/engine/javascript`) et jetons, jamais `codeToHtml` |
| `mermaid` | `11.17.2` | `12.0.0` | relevé le 2026-09-24 (11.17.2 publiée le 2026-08-25, 12.0.0 le 2026-09-10). **La 12 vise Safari 17.4+ et ES2024**, alors que la cible est macOS 13.0 (`minimumSystemVersion`), livré avec Safari 16 : on reste sur la dernière 11.x. À rouvrir quand la cible minimale garantit un WebKit 17.4 |

### Faits qui ont décidé du code

| Fait | Source | Vérifié le |
|---|---|---|
| Tauri ne sert que du statique : SSG, SPA ou MPA, pas de SSR | `https://v2.tauri.app/start/frontend/` | 2026-09-15 |
| Le mode SPA de Start écrit un shell (`/_shell.html` par défaut, `prerender.outputPath`) | `https://tanstack.com/start/latest/docs/framework/react/guide/spa-mode` | 2026-09-15 |
| Cibles de build du guide Vite de Tauri : `chrome105` sous Windows, `safari13` ailleurs | `https://v2.tauri.app/start/frontend/vite/` | 2026-09-15 |
| Tauri injecte nonces et hashes dans la CSP **aussi en développement** ; un nonce annule `'unsafe-inline'`, et les scripts inline de Vite sont bloqués : la fenêtre reste blanche sans erreur visible | `https://v2.tauri.app/reference/config/` et constat sur macOS 26.2 | 2026-09-15 |
| `@storybook/tanstack-react@10.6.0` embarque `@tanstack/router-core@1.171.30` ; avec `react-router@1.170.36`, toute story échoue sur `path.endsWith is not a function` | constat, `vitest --project storybook` | 2026-09-15 |
| Une commande sans `async` s'exécute **sur le thread principal**, sauf déclarée `#[tauri::command(async)]` ; une commande `async` ne peut prendre `State<'_, T>` qu'en renvoyant un `Result` | `https://v2.tauri.app/develop/calling-rust/` | 2026-09-15 |
| `@xyflow/react` met `pointer-events: none` sur un nœud ni sélectionnable ni déplaçable : un bouton dans le nœud ne reçoit plus le clic sans `pointer-events-auto` sur son contenu | constat, `vitest --project storybook`, 12.11.6 | 2026-09-24 |
| L'attribution de React Flow (un lien vers reactflow.dev) ne se retire, selon ses auteurs, qu'avec un abonnement React Flow Pro : Oxyn la garde | `https://reactflow.dev/learn/troubleshooting/remove-attribution` | 2026-09-24 |
| Le moteur par défaut de shiki est Oniguruma **compilé en WebAssembly** ; la CSP de production (`script-src 'self'`, sans `'wasm-unsafe-eval'`) le refuse. Le moteur JavaScript transpile les motifs en `RegExp` natives ; avec `target: 'auto'` (défaut) il n'emploie le drapeau `v` (ES2024) que si le moteur l'a, sinon le drapeau `u` | `https://shiki.style/guide/regex-engines` | 2026-09-24 |
| mermaid 12 : « built to target Safari 17.4+ and ES2024 » ; ELK devient la disposition par défaut | `https://github.com/mermaid-js/mermaid/releases/tag/mermaid%4012.0.0` | 2026-09-24 |
| mermaid 11.17.2 refuse qu'une directive ou l'en-tête d'un diagramme change une clé listée dans `secure` (par défaut `secure`, `securityLevel`, `startOnLoad`, `maxTextSize`, `suppressErrorRendering`, `maxEdges`), et assainit toute directive (`sanitizeDirective`) | sources de `mermaid@11.17.2`, `dist/chunks/mermaid.core/chunk-DU6HZSFF.mjs` | 2026-09-24 |
| Sous la CSP de `tauri.conf.json` servie en en-tête, un build de production (cible `safari13`) colore le code, dessine mermaid en `data:` URL, le diagramme des tables et le graphique **sans aucune violation**, dans Chromium et WebKit 26.6 (Playwright 1.63.0). Non reproduit : les hashes que Tauri ajoute lui-même à la CSP, et le WebKit de macOS 13 | constat, harnais `vite build` + Playwright | 2026-09-24 |
| `PathResolver::app_log_dir` : `home_dir/Library/Logs/<identifier>` sous macOS, `data_local_dir/<identifier>/logs` ailleurs, par le crate `dirs`. `logging::directory` le recalcule par `directories` (même `dirs-sys`) parce que le backend s'ouvre avant que l'application existe : à revérifier à chaque montée de `tauri` | sources de `tauri@2.11.5`, `src/path/desktop.rs` | 2026-09-24 |
| Sous macOS, tao traite `applicationWillTerminate` (→ `Event::LoopDestroyed` → `RunEvent::Exit`) mais pas `applicationShouldTerminate` : le Quit prédéfini du menu, celui du Dock et la fermeture de session terminent l'application sans `ExitRequested`, que rien ne peut retenir ([ADR-0038](adr/0038-un-plantage-s-annonce-une-fois.md)). `tao@0.37.0`, la dernière publiée, n'enregistre pas davantage `applicationShouldTerminate:`, et `tauri@2.11.5` ne l'ajoute pas ; la sortie de tao (`app.exit`) passe par `[NSApp stop:]`, pas par `terminate:` ([ADR-0040](adr/0040-inscrire-la-fermeture-d-une-sortie-forcee.md)) | sources de `tao@0.35.3` (`src/platform_impl/macos/app_delegate.rs`, `app_state.rs`), de `tao@0.37.0` (même fichier, tag `tao-v0.37.0`), de `tauri@2.11.5` et de `tauri-runtime-wry@2.11.4` (`src/lib.rs`) | 2026-09-25 |
| SQLite annule automatiquement la transaction ouverte d'une connexion qu'on ferme ; après certaines erreurs (dont `SQLITE_BUSY` et `SQLITE_INTERRUPT`), une transaction peut être annulée d'office, et seul `sqlite3_get_autocommit` le révèle ([ADR-0039](adr/0039-etat-de-transaction-d-une-session.md)) | `https://www.sqlite.org/c3ref/close.html`, `https://www.sqlite.org/c3ref/get_autocommit.html` | 2026-09-25 |
| `rusqlite@0.37.0` expose `Connection::is_autocommit` ; `sqlx-postgres@0.9.0` garde l'état de `ReadyForQuery` (`Idle`, `Transaction`, `Error`) privé — `PgConnection::in_transaction` est `pub(crate)` et confond `Error` avec `Idle` ; `sqlx-core@0.9.0` `Connection::is_in_transaction` ne compte que les transactions ouvertes par sqlx (`transaction_depth`), pas un `BEGIN` tapé ([ADR-0039](adr/0039-etat-de-transaction-d-une-session.md)) | sources installées : `rusqlite-0.37.0/src/lib.rs`, `sqlx-postgres-0.9.0/src/connection/mod.rs` et `src/message/ready_for_query.rs`, `sqlx-core-0.9.0/src/connection.rs` | 2026-09-25 |
| `sqlx` journalise le texte entier d'une requête sur la cible `sqlx::query` : en `debug` par défaut, en `warn` au-delà d'une seconde | sources de `sqlx-core@0.9.0`, `src/connection.rs` et `src/logger.rs` | 2026-09-24 |
| Le serveur du navigateur de Vitest part du port 63315 et hérite du `server` de la configuration Vite : avec `server.strictPort: true`, deux worktrees qui lancent les stories en même temps échouent sur « Port 63315 is already in use ». `browser.api: { strictPort: false }` rétablit le repli de Vite sur le port suivant ; `port: 0` ne sert à rien, Vitest remplace un port nul par 63315 | sources de `@vitest/browser@4.1.11` (`dist/index.js`, plugin `vitest:browser:config`) et de `vitest@4.1.11` (`resolveApiServerConfig`), et constat, deux `make front-tests` simultanés | 2026-09-24 |
| `esbuild` et `unrs-resolver` livrent leur binaire en dépendance optionnelle : leurs scripts d'installation sont refusés (`allowBuilds`) | `pnpm install`, pnpm 11.1.2 | 2026-09-15 |
| `LexicalTypeaheadMenuPlugin` pose `role="listbox"` et `aria-label="Typeahead menu"` sur son ancre **à chaque rattachement** — l'ancre est retirée puis remise à chaque frappe —, et laisse `aria-activedescendant` sur `typeahead-item-0` quand la liste se vide : axe échoue en `aria-valid-attr-value` et, sur une liste sans option, en `aria-required-children` | sources de `@lexical/react@0.51.0` (`shared/LexicalMenu.tsx`) et constat, `vitest --project storybook` | 2026-09-24 |

### Suivis amont

Ce qu'Oxyn attend d'une bibliothèque plutôt que de le contourner. Chaque ligne
porte la décision qui fait attendre ; elle se re-vérifie avec `/versions`.

| Attendu | État amont | Source | Vérifié le |
|---|---|---|---|
| `sqlx` expose les notices PostgreSQL (`NoticeResponse`) à l'appelant, par connexion — ce qui débloque l'onglet `Messages`. Décision du 2026-09-24 (audit, D13) : on attend, le driver ne passe pas à `tokio-postgres` | **Rien de livré.** Dernière version `0.9.0` (crates.io), celle de `Cargo.lock`. Sur `main` (`b54008a`, 2026-09-14), `sqlx-postgres/src/connection/stream.rs` décode toujours la notice pour la journaliser sur `sqlx::postgres::notice` et la jeter. `PgSeverity` est réexporté, `Notice` non. Le ticket [#3621](https://github.com/transact-rs/sqlx/issues/3621) « Expose a stream of `NoticeResponse`s from Postgres », ouvert le 2024-12-01, n'a ni commentaire ni PR liée. Le dépôt a quitté `launchbadge/sqlx` pour `transact-rs/sqlx` après la 0.9.0 | crates.io `/api/v1/crates/sqlx` ; API GitHub (ticket, recherche `NoticeResponse`, contenu de `main`) ; `CHANGELOG.md` de `main`, aucune entrée postérieure à 0.9.0 | 2026-09-25 |

## CI et livraison GitHub

Relevé le 2026-09-24 sur l'API GitHub (`/repos/<dépôt>/releases/latest`, puis
`/repos/<dépôt>/commits/<tag>` pour le SHA). Les workflows épinglent le SHA,
pas le tag : un tag se déplace, un SHA non.

| Fait | Valeur | Source | Vérifié le |
|---|---|---|---|
| `actions/checkout` | `v7.0.1`, `3d3c42e5aac5ba805825da76410c181273ba90b1`, publiée le 2026-07-20 | API GitHub | 2026-09-24 |
| `actions/setup-node` | `v7.0.0`, `820762786026740c76f36085b0efc47a31fe5020`, publiée le 2026-07-14 | API GitHub | 2026-09-24 |
| `pnpm/action-setup` | `v6.1.0`, `ea17c68df8912ef543352723c149a84f56e3d413`, publiée le 2026-09-05 ; lit `packageManager` via `package_json_file` quand `version` est absent | API GitHub, `action.yml` à ce SHA | 2026-09-24 |
| `Swatinem/rust-cache` | `v2.9.2`, `6323deb102c322ba6fcbdcafc7e3dddab59af2b6`, publiée le 2026-08-06 | API GitHub | 2026-09-24 |
| `Swatinem/rust-cache` n'enregistre le cache qu'après un job réussi, sauf `cache-on-failure: true` (`post-if: "success() \|\| env.CACHE_ON_FAILURE == 'true'"`) | `action.yml` et `src/restore.ts` à ce SHA | 2026-09-24 |
| `actions/cache` | `v6.1.0`, `55cc8345863c7cc4c66a329aec7e433d2d1c52a9`, publiée le 2026-06-26 ; n'enregistre qu'après un job réussi (`post-if: "success()"`) | API GitHub, `action.yml` à ce SHA | 2026-09-24 |
| Node exigé par le front | vite 8.3.0 : `^20.19.0 \|\| >=22.12.0` ; vitest 4.1.11 : `^20.0.0 \|\| ^22.0.0 \|\| >=24.0.0` ; la CI prend la ligne 22, celle de la machine de dev (22.23.2) | champ `engines` des paquets installés | 2026-09-24 |
| Bibliothèques système de Tauri sous Debian/Ubuntu | `libwebkit2gtk-4.1-dev build-essential curl wget file libxdo-dev libssl-dev libayatana-appindicator3-dev librsvg2-dev` | `tauri-apps/tauri-docs`, branche `v2`, `src/content/docs/start/prerequisites.mdx`, commit `2e513e3` du 2026-08-20 | 2026-09-24 |
| `keyring` 4.2.0 sous Linux passe par `zbus-secret-service-keyring-store` et `secret-service` 5.2.0 : du Rust pur, sans `libdbus` | `cargo tree --target x86_64-unknown-linux-gnu` | 2026-09-24 |
| `https://get.nexte.st/latest/linux` redirige vers `cargo-nextest-0.9.146-x86_64-unknown-linux-gnu.tar.gz` | en-tête `location` de la réponse | 2026-09-24 |
| La CI échouait à chaque poussée depuis au moins le 2026-09-21, en une vingtaine de secondes : pnpm absent du runner, `make qualite` s'arrêtait avant le front | journal du run `36034960589` | 2026-09-24 |
| Aucune des exécutions de la porte n'avait enregistré de cache Cargo : toutes échouaient, et l'étape « Post Restaurer le cache Cargo » était `skipped`. Seuls les caches pnpm existaient. Sous Linux, les stories prenaient 275 s (1045 tests) avant l'échec | runs `36037137682` et `36036585873`, `GET /actions/caches` | 2026-09-24 |
| La protection de branche est refusée sur ce dépôt : « Upgrade to GitHub Pro or make this repository public » (403). Rien n'empêche donc de fusionner une PR dont la CI échoue | `GET /repos/so-keyldzn/oxyn/branches/main/protection` | 2026-09-24 |
| Plus aucun job ne démarrait sur les PR #31, #32 et #33 : « The job was not started because recent account payments have failed or your spending limit needs to be increased » (0 étape, aucun runner). La matrice macOS + Linux sur chaque PR, avec 4 jobs macOS par PR, avait épuisé le quota. D'où Linux seul sur les PR (`qualite.yml`) | annotations des jobs du run `36046972599` | 2026-09-24 |

## GPUI

| Fait | Valeur | Source | Vérifié le |
|---|---|---|---|
| Dernière version publiée | `0.2.2`, publiée le 2025-10-22 | crates.io API | 2026-09-05 |
| Licence | Apache-2.0 | crates.io API | 2026-09-05 |
| Édition | 2024 | crates.io API | 2026-09-05 |
| MSRV déclaré | **aucun** (`rust_version` absent) | crates.io API | 2026-09-05 |
| Dépendances normales non optionnelles | 65 | crates.io API `/dependencies` | 2026-09-05 |
| Features | `default`, `inspector`, `leak-detection`, `macos-blade`, `runtime_shaders`, `screen-capture`, `test-support`, `wayland`, `windows-manifest`, `x11` | crates.io API | 2026-09-05 |

> **Deux pièges vérifiés.**
> 1. La dernière publication remonte à **près de onze mois** alors que le
>    développement continue dans le dépôt amont. La version épinglée ne recevra
>    ni correctif ni nouvelle API. C'est un coût accepté, tranché en
>    [ADR-0009](adr/0009-source-dependance-gpui.md).
> 2. `gpui` épingle plusieurs de ses dépendances avec `=` — dont
>    `cocoa =0.26.0`, `cocoa-foundation =0.2.0`, `core-foundation =0.10.0`.
>    Une dépendance d'Oxyn sur une autre version de ces crates ne se résout pas :
>    Cargo échoue au lieu d'unifier. À vérifier avant d'ajouter toute crate qui
>    touche aux API système macOS.

### Le harnais de test de GPUI

Relevé dans les sources de `gpui 0.2.2` telles que publiées sur crates.io — la
feature `test-support` n'est pas documentée sur docs.rs, qui construit avec les
features par défaut.

| Fait | Valeur | Source | Vérifié le |
|---|---|---|---|
| Feature à activer | `test-support` — tire `leak-detection`, `rand`, `collections/test-support`, `util/test-support`, `http_client/test-support`, `wayland`, `x11` | `Cargo.toml` de la crate publiée | 2026-09-07 |
| Coût réel sur macOS | `wayland` et `x11` sont déclarées sous `[target.'cfg(any(target_os = "linux", target_os = "freebsd"))'.dependencies]` : n'ajoute que `rand` et `backtrace` | `Cargo.toml`, sections `[target.…]` | 2026-09-07 |
| Macro | `#[gpui::test]`, réexportée depuis `gpui_macros` | `src/gpui.rs:81` | 2026-09-07 |
| Contextes | `TestAppContext`, `VisualTestContext` | `src/app/test_context.rs` | 2026-09-07 |
| Simulation | `draw`, `simulate_click`, `simulate_mouse_down/up/move`, `simulate_keystrokes`, `simulate_input`, `simulate_modifiers_change`, `simulate_resize`, `simulate_prompt_answer`, `dispatch_action`, `run_until_parked` | `src/app/test_context.rs` | 2026-09-07 |
| Plateforme | `TestPlatform` — aucune fenêtre, aucun GPU, aucun serveur d'affichage requis | `src/platform/test/platform.rs` | 2026-09-07 |
| Système de texte | `NoopTextSystem` — police fictive : `advance = 600 × glyph_id`, `glyph_id = ch.len_utf16()`, `rasterize_glyph` rend un buffer vide | `src/platform.rs:594` | 2026-09-07 |

> **Ce que le harnais ne mesure pas.** `NoopTextSystem` rend les métriques de
> texte déterministes et fausses, et aucun pixel n'est produit. Donc : pas de
> capture d'image, pas de comparaison de rendu, et **aucune assertion valable sur
> une dimension qui dépend de la largeur d'un texte**. Un tel test est vert quelle
> que soit l'interface réelle. Conséquence pour les tests :
> [tests.md](../.claude/rules/tests.md#les-tests-dinterface).

## Ressources de l'interface Figma

Sources vérifiées le **2026-09-07** lors de l'intégration GPUI :

| Ressource | Source figée | Usage |
|---|---|---|
| Hugeicons Stroke Rounded | [Dépôt source](https://github.com/hugeicons/hugeicons-static/tree/f9dbcca8d72cc2777a0ccd873c274d9bf7a153e6), contours exportés du [Figma Oxyn](https://www.figma.com/design/Yviemi4brBczzdRdBp1ONv/Oxyn?node-id=13-291) | Douze SVG de 16 × 16 px, octets exacts embarqués ; notice amont conservée |
| Marque Oxyn | Même Figma, composant `149:22199`, relu après la mise à jour des couleurs | Deux SVG 32 × 32 px selon le thème, marges internes conservées ; fragment orangé à la source, recoloré en vert-de-gris le 2026-09-23, tracés inchangés ([marque](../assets/brand/README.md#couleurs)) |
| Geist | [vercel/geist-font](https://github.com/vercel/geist-font/tree/10dc7658f13c38a474cde201bb09a4617267545b/fonts/Geist/ttf) | Regular, Medium et SemiBold, TTF embarqués sous SIL OFL |

Nœuds, dimensions et SHA-256 : icônes dans `assets/ui/provenance.json` et
polices dans `assets/fonts/provenance.json`, retirés du dépôt, lisibles au commit `8a1b7ff`. Les notices de licence restent avec
les ressources. Les fichiers ne sont pas chargés depuis Figma au démarrage :
`UiAssets` rend les octets inclus à la compilation et les polices sont
enregistrées avant l'ouverture de la fenêtre.

## Crates candidates

Relevées au registre, non encore adoptées. Aucune n'entre dans le dépôt sans
passer par [`/adr`](../.claude/commands/adr.md) si elle engage l'architecture.

| Crate | Dernière stable | Publiée le | Vérifié le |
|---|---|---|---|
| `tokio` | `1.53.1` | 2026-07-20 | 2026-09-05 |
| `sqlx` | `0.9.0` | 2026-05-21 | 2026-09-05 |
| `rusqlite` | `0.40.2` | 2026-08-08 | 2026-09-05 |
| `duckdb` | `1.10505.0` | 2026-07-22 | 2026-09-05 |
| `mongodb` | `3.9.0` | 2026-09-03 | 2026-09-05 |
| `redis` | `1.6.0` | 2026-08-15 | 2026-09-05 |
| `serde` | `1.0.229` | 2026-07-18 | 2026-09-05 |
| `thiserror` | `2.0.20` | 2026-08-08 | 2026-09-05 |
| `anyhow` | `1.0.104` | 2026-07-18 | 2026-09-05 |
| `tracing` | `0.1.44` | 2025-12-18 | 2026-09-05 |
| `criterion` | `0.8.2` | 2026-02-04 | 2026-09-10 |
| `gpui` | `0.2.2` | 2025-10-22 | 2026-09-05 |

> `gpui` figure ici pour être couverte par le vérificateur automatique ; elle est
> adoptée, non candidate — voir la section GPUI ci-dessus et
> [ADR-0009](adr/0009-source-dependance-gpui.md).

> `criterion` est adoptée depuis le 2026-09-10, en **`[dev-dependencies]`
> seulement** — voir la section « Bancs d'essai » ci-dessous. Elle reste dans
> ce tableau pour être couverte par le vérificateur automatique.

> `duckdb` versionne en suivant la version amont de DuckDB (`1.10505.0`), pas en
> semver Rust classique. Ne pas déduire une rupture d'API d'un saut de majeure.

## Codex — contexte étendu

Vérifié le **2026-09-07** pour la configuration locale
[.codex/config.toml](../.codex/config.toml).

| Fait | Valeur | Source | Vérifié le |
|---|---|---|---|
| Modèle et fenêtre maximale annoncée | `gpt-6-astra`, 1 050 000 tokens | [Fiche officielle](https://developers.openai.com/api/docs/models/gpt-6-astra) | 2026-09-07 |
| Réglages du contexte | `model_context_window` et `model_auto_compact_token_limit` | [Référence officielle](https://learn.chatgpt.com/docs/config-file/config-reference) | 2026-09-07 |
| Chargement local | `.codex/config.toml`, uniquement pour un projet approuvé ; les options de lancement ont priorité | [Configuration officielle](https://learn.chatgpt.com/docs/config-file/config-basic) | 2026-09-07 |
| Client installé lors de la vérification | `codex-cli 0.153.4` | `codex --version` | 2026-09-07 |
| Catalogue local observé avant surcharge | 272 000 tokens, 95 % utilisables ; session existante à 258 400 tokens | `~/.codex/models_cache.json`, événement `token_count` de la session | 2026-09-07 |

Choix du projet : effort `high`, fenêtre déclarée de 1 050 000 tokens et
compactage à 700 000 tokens pour garder une marge aux réponses, au raisonnement
et aux retours d'outils. Ce seuil est un choix local, pas une limite officielle.
La fenêtre effective dépend du client et du service ; écrire cette valeur ne
prouve pas qu'une requête de cette taille a été acceptée. Avec la réserve locale
observée de 5 %, la fenêtre utilisable attendue est de 997 500 tokens.

Pour une utilisation facturée à l'API, la fiche du modèle annonce au-delà de
272 000 tokens d'entrée un multiplicateur de 2 sur l'entrée et le cache, et de
1,5 sur la sortie pour la requête entière. Ne pas extrapoler ces tarifs aux
quotas d'un abonnement ChatGPT.

Réglages complémentaires vérifiés le **2026-09-07** :

| Fait | Décision locale | Source |
|---|---|---|
| Cache de prompts activé par défaut sur les modèles compatibles | Laisser le service gérer le cache ; aucune clé `prompt_cache`, `prompt_cache_key` ou `prompt_cache_retention` de premier niveau documentée pour le fichier Codex | [Cache API](https://developers.openai.com/api/docs/guides/prompt-caching), [référence Codex](https://learn.chatgpt.com/docs/config-file/config-reference) |
| `web_search = "live"` permet la recherche web en direct | Vérifier les sources actuelles conformément à I-12 ; le mode web `cached` est indépendant du cache de prompts | [Référence Codex](https://learn.chatgpt.com/docs/config-file/config-reference) |
| `tui.status_line` configure la barre de la CLI | Afficher `model-with-reasoning`, `context-remaining`, `git-branch` | [Exemple officiel](https://learn.chatgpt.com/docs/config-file/config-sample) |

Le cache réutilise des préfixes identiques : garder les instructions stables et
continuer une même tâche dans son fil favorise cette réutilisation, sans la
garantir. Un succès de cache réduit le travail de traitement ; il ne retire pas
les tokens de la fenêtre de contexte. Les paramètres de rétention et de routage
documentés pour l'API ne doivent pas être transposés en clés Codex inventées.

## Codex — agents locaux et MCP

Vérifié le **2026-09-07** pour [.codex/](../.codex/README.md).

| Fait | Décision locale | Source |
|---|---|---|
| Les agents de projet sont découverts dans `.codex/agents/*.toml` ; `name`, `description` et `developer_instructions` sont requis | Onze profils courts renvoient aux guides communs et aux adaptations d'AGENTS.md | [Agents personnalisés](https://learn.chatgpt.com/docs/agent-configuration/subagents#custom-agents) |
| Les réglages de modèle et d'effort omis héritent du contexte de lancement | Aucune surcharge de modèle dans les profils | [Sous-agents](https://learn.chatgpt.com/docs/agent-configuration/subagents) |
| `agents.max_concurrent_threads_per_session` borne les sous-agents simultanés, hors agent principal | Trois sous-agents au maximum ; choix local, pas une limite du service | [Référence de configuration](https://learn.chatgpt.com/docs/config-file/config-reference) |
| Un profil peut déclarer `sandbox_mode`, mais les surcharges actives du parent peuvent primer | Défaut `read-only` pour les quatre relecteurs ; conserver aussi la consigne de ne rien modifier | [Permissions des sous-agents](https://learn.chatgpt.com/docs/agent-configuration/subagents#approvals-and-sandbox-controls) |
| `mcp_servers.<id>.required = false` laisse le serveur facultatif au démarrage | Conserver les deux déclarations Figma préexistantes sans exiger leur disponibilité | [Référence de configuration](https://learn.chatgpt.com/docs/config-file/config-reference) |

La délégation reste soumise à la demande et aux consignes d'AGENTS.md. Les
profils n'installent aucun hook Claude et ne modifient pas la configuration
globale. La disponibilité et l'authentification Figma doivent être vérifiées
dans la session qui utilise le service.

## Écosystème MCP

| Fait | Valeur | Source | Vérifié le |
|---|---|---|---|
| Serveurs de référence maintenus | `everything`, `fetch`, `filesystem`, `git`, `memory`, `sequential-thinking`, `time` | `github.com/modelcontextprotocol/servers` | 2026-09-05 |
| Serveurs de référence **archivés** | `postgres`, `sqlite`, `github` | idem | 2026-09-05 |

> Conséquence directe : il n'existe **aucun** serveur MCP officiel pour
> PostgreSQL ni SQLite. Tout serveur de base de données branché sur Oxyn serait
> un serveur tiers, à auditer. Le raisonnement complet est dans
> [MCP.md](MCP.md).

## Jetons de la maquette Figma

Fichier `Yviemi4brBczzdRdBp1ONv`, collection `Oxyn / Primitives`. Lus par le
serveur MCP Figma Dev Mode local (`get_metadata`, `get_variable_defs`), pas
recopiés d'une capture. Les couleurs et les icônes ont leur propre provenance,
plus détaillée, dans `assets/ui/provenance.json`, retiré du dépôt, lisible au commit `8a1b7ff`.

Les dimensions, espacements et rayons sont publiés dans
`crates/oxyn-ui/src/theme.rs` — `Metrics`, `Spacing`, `Radii` — où chaque champ
cite son nœud. **La typographie ne l'est que partiellement** : `Typography` ne
porte ni graisses ni styles nommés, et deux de ses valeurs restent sans source —
voir la table des manques ci-dessous.

| Fait | Valeur | Source | Vérifié le |
|---|---|---|---|
| Échelle d'espacement | `space/0`=0, `space/4`=4, `space/8`=8, `space/12`=12, `space/16`=16, `space/24`=24 | Nœuds `13:291` et `47:8222` | 2026-09-07 |
| Rayons de coin | `radius/6`=6, `radius/8`=8, `radius/full`=999 | idem | 2026-09-07 |
| Hauteur de barre d'outils | `48` | Nœud `47:8422` « Connection toolbar », confirmé par `190:1543`, `229:7640`, `232:9103` | 2026-09-15 |
| Largeur du panneau latéral déplié | `280` | Nœuds `8:4`, `13:292` | 2026-09-07 |
| Largeur du panneau latéral replié | `64` | Nœuds `13:165`, `13:484` | 2026-09-07 |
| Hauteur d'un contrôle | `38` | Nœuds `47:8461`, `47:8465`, `47:8469` | 2026-09-07 |
| Familles et graisses | Geist — Title 24/32 SemiBold, Body 13/20 Regular, Label 13/20 Medium, Caption 11/16 Regular, Section 11/16 Medium | Nœuds `13:291`, `47:8222` | 2026-09-07 |

> **Correction du 2026-09-15 — la hauteur de barre d'outils valait `52`.**
> Le nœud `47:8422` avait été lu sous le nom « Workspace toolbar » le
> 2026-09-07 ; relu au serveur, il s'appelle « Connection toolbar » et mesure
> **48**. Les trois planches de workspace le confirment sans exception. Le `52`
> ne venait donc d'aucune frame.
>
> Ce qui rend le cas instructif, et qui est exactement le mode de panne
> qu'[I-12](../CLAUDE.md#i-12) décrit : le dépôt avait pourtant tout ce qu'il
> fallait pour s'en apercevoir — la valeur était documentée, sourcée, datée, et
> un test de `theme.rs` l'ancrait. Le test était vert, sur une valeur fausse.
> Rien ne rougissait parce que **le champ n'avait aucun lecteur** : la barre de
> connexion se dessinait avec un `48` écrit en dur dans la vue. Deux sources
> pour une même mesure, dont une fausse et l'autre invisible au thème. La vue
> lit désormais `Metrics::toolbar_height`.

### Les binaires embarqués et leurs licences

Ces fichiers sont liés au binaire par `include_bytes!` dans
`crates/oxyn-ui/src/icons.rs`. Ils portent un commit amont exact, comme une
dépendance de code.

| Fait | Valeur | Source | Vérifié le |
|---|---|---|---|
| Police d'interface | Geist Regular, Medium, SemiBold, commit `10dc7658f13c38a474cde201bb09a4617267545b` | [vercel/geist-font](https://github.com/vercel/geist-font) ; SHA-256 dans `assets/fonts/provenance.json`, retiré du dépôt, lisible au commit `8a1b7ff` | 2026-09-07 |
| Licence de la police | SIL Open Font License 1.1 | `assets/fonts/OFL.txt`, `assets/fonts/LICENSE.txt` | 2026-09-07 |
| Icônes | Hugeicons Stroke Rounded, 12 glyphes, commit `f9dbcca8d72cc2777a0ccd873c274d9bf7a153e6` | [hugeicons/hugeicons-static](https://github.com/hugeicons/hugeicons-static) ; SHA-256 dans `assets/ui/provenance.json`, retiré du dépôt, lisible au commit `8a1b7ff` | 2026-09-07 |
| Licence des icônes | **Aucune licence MIT attribuée** ; le README amont autorise l'usage tel quel, sans mention d'un droit de redistribution | `assets/ui/HUGEICONS-UPSTREAM-README.txt`, retiré du dépôt, lisible au commit `8a1b7ff` | 2026-09-07 |

> **À trancher avant la première publication de binaire.** Oxyn redistribue ces
> douze glyphes en les liant dans l'exécutable. Tant qu'aucune version n'est
> publiée, la question ne se pose pas ; elle se posera d'un coup le jour de la
> première release, et c'est une question de droit, pas de code.

### Ce qui n'a pas pu être lu, et pourquoi

Le serveur Dev Mode applique un **quota journalier**, épuisé le 2026-09-07 par un
balayage d'identifiants de pages. Sont donc restés non vérifiés, et ne doivent
pas être considérés comme sourcés tant qu'ils ne sont pas relus :

| Non lu | Où le chercher | Conséquence dans le code |
|---|---|---|
| Hauteur de barre d'état | pages `03 · Foundations` ou `22 · Database workspace` | `Metrics::status_bar_height` vaut 32, valeur de consigne **non confirmée** |
| Espacements 20 et 32 | idem | **non publiés** : absents des deux écrans lus, qui n'emploient que 0/4/8/12/16/24 |
| Métriques de grille — hauteur de ligne et d'en-tête, largeurs de colonne, gouttière | page `22 · Database workspace` | les valeurs préexistantes de `Metrics` sont conservées telles quelles, sans être attribuées à la maquette |
| Épaisseur de l'anneau de focus | composant `Focus 11:69` | `Metrics::focus_ring` vaut 2, justifié par la lisibilité et non par la maquette |
| Interligne de l'éditeur à chasse fixe | page `03 · Foundations` | `Typography::line_height` vaut 18 : ni 20 ni 16, les deux interlignes lues ; c'est une valeur d'éditeur, à ne pas « corriger » d'après la ligne de typographie ci-dessus |
| Famille à chasse fixe | idem | `Typography::mono_family` vaut `Menlo`, une police système macOS ; la maquette n'a pas pu être consultée sur ce point |

Les identifiants des pages `03 · Foundations` et `22 · Database workspace` n'ont
pas été retrouvés : `get_metadata` exige un nœud connu, et les pages ne
s'énumèrent pas. Les obtenir demande de les ouvrir dans l'application Figma, ou
de lire l'URL `?node-id=` de chacune.

## Aperçus PostgreSQL et fuseaux Arrow

Vérifié le 2026-09-07 dans les sources résolues par `Cargo.lock` et les sources
amont :

- [Arrow : fuseaux](https://docs.rs/arrow-array/latest/arrow_array/timezone/struct.Tz.html) :
  sans la fonctionnalité `chrono-tz`, seuls les décalages fixes sont acceptés.
  Le driver fournit `UTC` pour `timestamptz` ; le workspace active donc cette
  fonctionnalité pour la grille et les exports. Cargo résout la dépendance
  transitive [chrono-tz 0.10.4](https://crates.io/crates/chrono-tz/0.10.4), sans
  changer la version d'Arrow.
- [PostgreSQL : pg_type](https://www.postgresql.org/docs/17/catalog-pg-type.html) :
  `typsend = 0` indique l'absence de sortie binaire ; la catégorie `Z` désigne
  les types internes. Le décodeur de catégories de SQLx installé refuse `Z`,
  ce qui bloque notamment `pg_node_tree` avant même la lecture des lignes.
- [PostgreSQL : alias d'OID](https://www.postgresql.org/docs/17/datatype-oid.html) :
  la sortie texte de `regproc` et des autres alias expose les noms d'objets.
  L'aperçu demande ce rendu au serveur ; des octets binaires valides en UTF-8
  ne constituent pas une représentation textuelle fiable.

## `rustls` — avis de sécurité du 2026-09-14

`cargo deny` a signalé **RUSTSEC-2026-0285** sur `rustls 0.23.43`, tiré par
`reqwest 0.13.4` via `hyper-rustls 0.27.9` pour `oxyn-llm`.

L'avis : rustls acceptait des messages de handshake TLS 1.3 envoyés au **mauvais
niveau de chiffrement** lorsqu'ils suivaient un message changeant de clé dans le
même enregistrement — un `EncryptedExtensions` en clair empaqueté avec le
`ServerHello`, par exemple. RFC 8446 §5.1 exige de terminer la connexion par une
alerte `unexpected_message`. Même défaut fonctionnel que
[GO-2026-4340](https://pkg.go.dev/vuln/GO-2026-4340) (CVE-2025-61730).

**Portée réelle, telle que l'avis la décrit** : la transcription du handshake
reste authentifiée, donc un attaquant en position réseau ne peut ni l'altérer ni
le compléter. L'effet pratique est qu'un pair pouvait envoyer en clair des
messages qui auraient dû être chiffrés sans que rustls refuse la connexion.

Corrigé par `cargo update -p rustls` : **0.23.43 → 0.23.45**, mise à jour de
correctif dans la même plage sémantique. Aucun manifeste modifié, seul
`Cargo.lock`. Vérifié le 2026-09-14 : `cargo deny check advisories` ne signale
plus rien.

## Agent Client Protocol — vérification du 2026-09-14

Piste ouverte par l'utilisateur : l'intégration IA, surtout pour ne pas passer
par les API ; « il y a deux modes, un avec API et l'autre les agents
externes ». Vérification faite aux sources, pas de mémoire.

**Le protocole.** L'Agent Client Protocol est du **JSON-RPC**, sur `stdio` pour
un agent local, sur HTTP ou WebSocket pour un agent distant. Un agent local est
un **processus enfant de l'éditeur**. Le tour de dialogue est documenté avec ses
méthodes : `session/prompt` (client → agent) ouvre le tour, `session/update`
(agent → client) diffuse les fragments — `agent_message_chunk`, `plan`,
`tool_call`, `tool_call_update`, `usage_update` —, `session/request_permission`
(**agent → client**) demande l'autorisation avant d'exécuter un outil, et
`session/cancel` (client → agent) interrompt. Le tour se termine par une réponse
portant un `StopReason` : `end_turn`, `max_tokens`, `max_turn_requests`,
`refusal` ou `cancelled`. Consultation d'[agentclientprotocol.com](https://agentclientprotocol.com/protocol/prompt-turn)
le 2026-09-14.

**La bibliothèque.** `agent-client-protocol` **2.1.0** sur crates.io, publiée le
2026-09-04, sous **Apache-2.0** — licence déjà acceptée par `deny.toml`.
`rust-version` déclaré **1.88.0**, édition **2024** : compatible avec la chaîne
épinglée du dépôt (1.98.1) sans y toucher. Dépôt :
[agentclientprotocol/rust-sdk](https://github.com/agentclientprotocol/rust-sdk).
Le SDK expose les rôles `Client`, `Agent`, `Proxy` et `Conductor` avec des
constructeurs de connexion ; **le numéro de version majeur ne dit pas la version
du protocole** — la 2.1.0 porte le protocole v1 *stable* et un v2 *brouillon*,
ce dernier derrière `.v2()`. Les transports HTTP/SSE et WebSocket vivent dans une
crate séparée, `agent-client-protocol-http`, dont nous n'aurions pas besoin pour
un agent local.

**Ce que devient le processus enfant à l'annulation** — lu dans la source de la
crate le **2026-09-15**, parce que toute l'annulation d'un tour d'agent en
dépend et que l'affirmation circulait sans preuve. Dans
`agent-client-protocol-2.1.0/src/acp_agent.rs` :

| Fait | Où |
|---|---|
| L'enfant est lancé **chef de son propre groupe** (`std_cmd.process_group(0)`) — tuer le groupe n'atteint donc pas Oxyn | `spawn_process` |
| `ChildGuard::terminate` envoie `SIGKILL` au **groupe** (`rustix::process::kill_process_group`) puis `kill()` en secours, ce qui atteint les petits-enfants d'un lanceur `npx` ou `uvx` | `ChildGuard` |
| Le garde est construit **avant le premier `poll`** : « Create the guard eagerly so cancelling this connection before the monitor is first polled still terminates the whole process group » | à la création de `child_wait` |

Conséquence retenue : **abandonner le futur de conversation suffit** à terminer
l'agent, y compris si l'annulation arrive avant que quoi que ce soit n'ait été
lu. C'est ce sur quoi repose `run_turn`, qui sélectionne la conversation contre
le jeton d'annulation plutôt que de relire un drapeau entre deux étapes.

**Ce qu'en fait un éditeur client.** Un agent externe s'y déclare par une
commande, ses arguments et son environnement, et l'éditeur le lance en
processus séparé. **Aucune clé d'API n'est requise pour un agent externe**, qui
porte sa propre authentification ; la facturation, les conditions et la
rétention des données regardent l'utilisateur et le fournisseur de l'agent.
Cela s'oppose aux fournisseurs natifs, où la clé est configurée dans
l'éditeur. Relevé le 2026-09-14.

**Ce que la crate ajoute au processus**, mesuré le 2026-09-14 par
`cargo tree -p oxyn-ai --edges normal -i <crate>` : `async-io 2.6.0`,
`async-process 2.5.0`, `async-signal 0.2.14` et `blocking 1.7.0`. Tokio n'est
qu'une dépendance **de développement** de la crate — son cœur est `futures`,
agnostique. En revanche `async-io` démarre un fil de réacteur et `blocking` un
pool : **deux réacteurs cohabitent** avec celui de Tokio. `smol`,
`async-executor` et `async-global-executor` apparaissent dans `Cargo.lock` mais
**pas** dans le graphe normal d'`oxyn-ai` — ils viennent de dépendances de
développement d'ailleurs, et la distinction vaut d'être faite : lire le
`Cargo.lock` seul aurait fait conclure à un runtime complet de plus.

Le chemin `ConnectTo` de la crate installe un garde qui termine le **groupe de
processus** (`process_group(0)` sur Unix), et non le seul enfant : un agent
distribué derrière `npx` ou `uvx` se ré-attacherait sinon à pid 1 et ne
s'arrêterait pas de façon fiable sur EOF de son entrée standard.

Ce que le dépôt en déduit est décidé dans
[ADR-0026](adr/0026-agents-externes-acp.md), pas ici.

### Adaptateurs ACP de Claude Code et de Codex — vérification du 2026-09-15

Demande de l'utilisateur : se connecter à Claude par **Claude Code déjà installé
et authentifié** (son abonnement, aucune clé confiée à Oxyn), et de même pour
**Codex**. Aucun des deux ne parle ACP nativement : chacun passe par un
adaptateur. Relevé au registre npm, dans le registre ACP, dans la source des
adaptateurs et dans la documentation officielle, le **2026-09-15** ; ce sont les
valeurs de `crates/oxyn-ai/src/external/presets.rs`.

| Paquet | Version | Licence | `bin` | Source |
|---|---|---|---|---|
| `@agentclientprotocol/claude-agent-acp` | **0.78.0** | Apache-2.0 (le registre ACP écrit « proprietary », voir plus bas) | `claude-agent-acp` ; `engines` : `node >=22` | [registre npm](https://registry.npmjs.org/@agentclientprotocol/claude-agent-acp), [registre ACP](https://github.com/agentclientprotocol/registry/blob/main/claude-acp/agent.json) |
| `@anthropic-ai/claude-agent-sdk` (dépendance, CLI embarquée 2.1.270) | 0.3.270 | « SEE LICENSE IN README.md » | — | [registre npm](https://registry.npmjs.org/@anthropic-ai/claude-agent-sdk/0.3.270) |
| `@agentclientprotocol/codex-acp` | **1.12.0** | Apache-2.0 | `codex-acp` ; aucun `engines` déclaré, mais sa dépendance `open@^11` exige `node >=20`, la plus haute de ses dépendances directes (`vscode-jsonrpc@9` : `>=14`, `diff@9` : `>=0.3.1`, `zod@4` et `@agentclientprotocol/sdk@1.4` : rien) — relevé au registre npm le 2026-09-23 | [registre npm](https://registry.npmjs.org/@agentclientprotocol/codex-acp), [registre ACP](https://github.com/agentclientprotocol/registry/blob/main/codex-acp/agent.json) |
| `@openai/codex` (dépendance) | 0.154.0 | Apache-2.0 | `codex` ; `node >=16` | [registre npm](https://registry.npmjs.org/@openai/codex/latest) |

Les deux adaptateurs publient plusieurs fois par semaine (les deux dernières
versions datent du 2026-09-15 à sept minutes d'écart) : la commande proposée
**épingle** la version, `npx -y <paquet>@<version>`, forme déclarée par le
registre ACP.

**Connexion.** Relevé dans la source de chaque adaptateur (`src/acp-agent.ts`,
`src/CodexAuthMethod.ts`) et dans la documentation de chaque agent :

- Claude : l'adaptateur n'annonce **aucune** méthode si le client ne déclare pas
  `clientCapabilities.auth.terminal` ; il annonce sinon des méthodes `terminal`
  (`--cli auth login --claudeai`). La spécification réserve cette capacité au
  client qui « can reproduce the configured agent invocation in an interactive
  terminal » : Oxyn ne le peut pas et **ne la déclare pas**. Sans session, il
  renvoie `auth_required` (-32000). La connexion se fait dans un terminal par
  `claude auth login` ([référence de la CLI](https://code.claude.com/docs/en/cli-reference)) ;
  les identifiants vivent dans le trousseau macOS ou `~/.claude`
  ([authentification](https://code.claude.com/docs/en/authentication)), que
  l'adaptateur relit. **Déduit, non écrit** dans la documentation de
  l'adaptateur : une connexion faite avec le `claude` de l'utilisateur est
  réutilisée à configuration identique.
  **Sans `claude` installé — vérifié le 2026-09-23** dans la source publiée de
  0.78.0 (cache `npx`) : `dist/index.js` transmet tout ce qui suit `--cli` à la
  CLI que le SDK embarque (`claudeCliPath()`, `dist/acp-agent.js`), et les
  méthodes `terminal` qu'il annonce sont exactement
  `--cli auth login --claudeai` et `--cli auth login --console`, ajoutés à la
  commande qui le lance. Mesuré sur la machine de développement :
  `npx -y @agentclientprotocol/claude-agent-acp@0.78.0 --cli --version` répond
  `2.1.270 (Claude Code)`, et `… --cli auth login --help` liste `--claudeai`
  (« Use Claude subscription (default) »). Oxyn propose donc cette commande,
  composée avec la commande déclarée, quand `claude` est introuvable à l'écran
  des fournisseurs, et toujours dans le panneau pour la déclaration épinglée
  (il n'y cherche pas `claude`). Rien d'équivalent n'est vérifié pour
  `codex-acp` : `codex login` reste la seule proposition.
- Codex : méthodes de genre `agent`, qui passent par `authenticate` —
  `chat-gpt` (réussit aussitôt si un compte est déjà connecté, ouvre le
  navigateur sinon) et `api-key` (clé passée en `_meta` ou lue dans
  l'environnement, qu'Oxyn **ne propose pas** : il ne détient aucune clé
  d'agent). `session/new` rend `auth_required` sans compte ; la connexion en
  terminal est `codex login` ([authentification Codex](https://developers.openai.com/codex/auth)).

**Emplacements usuels** cherchés à l'ouverture de l'écran des fournisseurs
(et par « Detect again »), parce qu'une
application lancée depuis le Finder n'hérite pas du `PATH` du shell :
`~/.local/bin` (installateurs natifs de Claude Code et de Codex,
[installation Claude Code](https://code.claude.com/docs/en/setup),
[script d'installation Codex](https://raw.githubusercontent.com/openai/codex/main/scripts/install/install.sh)),
`~/.claude/local` (ancienne installation npm locale de Claude Code),
`/opt/homebrew/bin`, `/usr/local/bin`, `/home/linuxbrew/.linuxbrew/bin`
([Homebrew](https://docs.brew.sh/Installation)). Seules les entrées **absolues**
de `PATH` sont gardées : `.` ou `bin` se résoudraient contre le répertoire
courant d'Oxyn.

**nvm et Volta — vérification du 2026-09-23.** Ajoutés après qu'une machine
dont Node vient de nvm n'a pas trouvé `npx` depuis le Finder.

| Gestionnaire | Ce qui est relevé | Source |
|---|---|---|
| nvm **0.40.8** | installé dans `~/.nvm`, ou `${XDG_CONFIG_HOME}/nvm` si cette variable existe (`NVM_DIR`) | [README](https://github.com/nvm-sh/nvm/blob/v0.40.8/README.md), l. 120-126 |
| | une version vit dans `$NVM_DIR/versions/node/<version>`, son exécutable dans `<version>/bin` | [`nvm.sh`](https://github.com/nvm-sh/nvm/blob/v0.40.8/nvm.sh) : `nvm_version_dir` (l. 781-785), `NVM_NODE_PATH="${VERSION_PATH}/bin/…"` (l. 253) |
| | les alias sont des fichiers de `$NVM_DIR/alias` (`nvm_alias_path`, l. 796), ceux des LTS sous `alias/lts/` ; un alias peut viser un autre alias, nvm suit la chaîne et s'arrête sur un cycle (`nvm_resolve_alias`, l. 1553) ; `..` est refusé dans un nom (l. 1507-1510) | `nvm.sh` |
| | `default` peut valoir `node` (la plus récente installée), `18` (la plus récente v18.x), `18.12` (la plus récente v18.12.x) — « The first version installed becomes the default » | README, l. 396 et 626-628 |
| Volta | « The shim directory is at `$VOLTA_HOME/bin` », `VOLTA_HOME` valant `~/.volta` sur Unix | [installateurs Volta](https://docs.volta.sh/advanced/installers) |

Ce qu'Oxyn en fait (`crates/oxyn-ai/src/external/locate/nvm.rs`) : il suit
l'alias `default` sous `~/.nvm` et garde la version installée qu'il désigne
**si elle satisfait le minimum de l'agent** — `node >=22` pour `claude-agent-acp`,
`node >=20` pour `codex-acp` (ci-dessus), aucun pour un agent qu'Oxyn ne connaît
pas ; sinon la plus haute version installée qui le satisfait ; sinon rien. `NVM_DIR`,
`XDG_CONFIG_HOME` et `VOLTA_HOME` ne sont pas lus : c'est un profil de shell qui
les pose, et un processus qui en a exécuté un a déjà ces répertoires dans son
`PATH`. **Non vérifiés, donc non cherchés** : fnm (ses répertoires `multishell`
sont propres à chaque shell) et asdf.

**Écart à signaler.** Le registre ACP déclare la licence de Claude Agent
« proprietary » quand `package.json` et `LICENSE` disent Apache-2.0 ;
l'explication probable est la dépendance `@anthropic-ai/claude-agent-sdk`, sous
conditions Anthropic. Oxyn ne redistribue ni l'un ni l'autre : `npx` les
télécharge sur la machine de l'utilisateur.

### Exposer les outils d'Oxyn à un agent externe — vérification du 2026-09-16

Question : par quel transport un agent externe peut-il atteindre les outils
d'Oxyn ([ADR-0030](adr/0030-outils-oxyn-exposes-a-un-agent-externe.md)) ?

**Ce que la crate offre.** `agent-client-protocol` 2.1.0 déclare quatre
transports de serveur MCP dans `NewSessionRequest.mcp_servers`
(`agent-client-protocol-schema-1.7.0/src/v1/agent.rs:2628`) : `Stdio` — « All
Agents MUST support this transport » —, `Http`, `Sse`, et `Acp`. Seul `Acp`
porte le serveur **en mémoire**, sans processus ni port ; il est derrière la
feature `unstable_mcp_over_acp` et conditionné à une capacité annoncée par
l'agent (`McpCapabilities.acp`, même fichier, ligne 4537).

**Mesure, et non supposition.** Les deux adaptateurs, tels qu'installés sur la
machine de développement, ont été interrogés par un `initialize` ACP — sans
authentification ni accès à une base. Ce qu'ils annoncent :

| Adaptateur | Version mesurée | `mcpCapabilities` |
|---|---|---|
| `@agentclientprotocol/claude-agent-acp` | 0.78.0 | `{"http": true, "sse": true}` — `acp` **absent** |
| `@agentclientprotocol/codex-acp` | 1.12.0 | `{"acp": false, "http": true, "sse": false}` |

**Conséquence : le transport `Acp` est inutilisable aujourd'hui**, et le seul
transport accepté par les deux agents en plus de `stdio` est **`http`**.
`McpServerHttp` porte `name`, `url` et `headers`
(`.../schema-1.7.0/src/v1/agent.rs:2667`), donc un jeton porteur.

La mesure confirme au passage les versions relevées le 2026-09-15 : les binaires
en cache déclarent bien 0.78.0 et 1.12.0, et Codex annonce les méthodes
d'authentification `api-key` et `chat-gpt`, comme documenté plus haut.

**Le protocole MCP n'est pas dans la crate.** `agent-client-protocol` 2.1.0 ne
contient ni `initialize`, ni `tools/list`, ni `tools/call` : `McpToolRegistry`
fournit le catalogue et les schémas, rien ne les sert sur le fil. La crate
d'adaptation est nommée dans ses propres sources
(`agent-client-protocol-2.1.0/src/mcp_server/mod.rs:18`) :

| Crate | Version | Licence | Publiée | Exige |
|---|---|---|---|---|
| `agent-client-protocol-rmcp` | **3.1.0** | Apache-2.0 | 2026-09-04 | `agent-client-protocol ^2.1.0`, `rmcp ^2.1.0`, `tokio ^1.52`, `tokio-util ^0.7`, `schemars ^1.0` |

Relevé à [crates.io](https://crates.io/api/v1/crates/agent-client-protocol-rmcp)
le 2026-09-16.

**Elle n'est pas retenue.** Le transport `Acp` qu'elle sert est celui que les
agents n'acceptent pas (mesure ci-dessus), et sur un transport HTTP le protocole
MCP est à notre charge de toute façon. La consigner ici sert à ce que la
question ne soit pas reposée sans la mesure.

**Ce que le pont utilise à la place**, relevé à crates.io le **2026-09-16**.
`hyper`, `hyper-util` et `http-body-util` étaient déjà dans `Cargo.lock` par
`reqwest` et `tauri` : les déclarer ne fait entrer aucune nouvelle famille dans
l'arbre.

| Crate | Version | Licence | Pourquoi |
|---|---|---|---|
| `hyper` | **1.11.1** | MIT | l'écoute HTTP du serveur MCP, sur la boucle locale |
| `hyper-util` | **0.1.20** | MIT | le service et l'acceptation des connexions |
| `http-body-util` | **0.1.5** | MIT | lire et écrire un corps complet |
| `async-process` | **2.5.0** | Apache-2.0 OR MIT | lancer l'agent avec un environnement en liste blanche ; ses flux sont déjà des `futures::io`, donc aucun pont d'exécuteur |
| `rustix` | **0.38.44** | Apache-2.0 WITH LLVM-exception OR Apache-2.0 OR MIT | tuer le **groupe** de processus. La version maximale est `1.1.4` ; nous alignons sur `0.38.44`, celle qu'`agent-client-protocol` tire déjà, pour ne pas compiler deux copies d'une crate d'appels système |

**Les révisions MCP que le pont annonce**, relevées le **2026-09-16** dans le
paquet installé par l'adaptateur Claude, et non de mémoire. Le serveur les
**négocie** : il garde la révision demandée par l'agent quand elle figure dans la
liste, et répond la plus récente sinon (`crates/oxyn-ai/src/external/mcp.rs`,
`SUPPORTED_VERSIONS`).

| Source | Version | Licence | Où |
|---|---|---|---|
| `@modelcontextprotocol/sdk` | **1.30.0** | MIT | `dist/esm/types.js:4` et `dist/cjs/types.js:33-35`, dans le cache `npx` de `@agentclientprotocol/claude-agent-acp` 0.78.0 ; dépendance *peer* `^1.29.0` de `@anthropic-ai/claude-agent-sdk` 0.3.270 |

`LATEST_PROTOCOL_VERSION = '2025-11-25'`, puis
`SUPPORTED_PROTOCOL_VERSIONS = [LATEST, '2025-06-18', '2025-03-26', '2024-11-05', '2024-10-07']`.

**Non mesuré : les révisions de Codex 1.12.0.** L'adaptateur Codex n'embarque pas
ce paquet JavaScript. La négociation couvre le cas d'une révision qu'Oxyn ne
connaît pas (il répond la sienne, l'agent décide), mais une révision **connue**
dont les exigences de transport différeraient — un flux sur `GET`, par exemple —
ne serait pas détectée. À vérifier avec la prochaine montée de version de l'un
ou l'autre adaptateur.

### Délai d'un appel d'outil MCP côté agent — relevé du 2026-09-24

Un appel `execute_query` dont l'écriture attend l'accord de l'utilisateur reste
suspendu jusqu'à la décision, cinq minutes au plus (la durée de vie d'une
demande, `oxyn_exec::approval::DEFAULT_TTL`). La question est de savoir si
l'agent, de son côté, abandonne l'appel avant. Relevé dans les paquets
téléchargés par `npm pack` le 2026-09-24, et dans les sources de Codex à
l'étiquette correspondante — jamais de mémoire :

| Agent | Ce que l'adaptateur transmet du serveur MCP d'Oxyn | Délai appliqué à un `tools/call` | Source |
|---|---|---|---|
| `@agentclientprotocol/claude-agent-acp` **0.78.0** | `type`, `url`, `headers` — aucun `timeout` | celui de `@anthropic-ai/claude-agent-sdk` **0.3.270** : `timeout` du serveur, sinon la variable `MCP_TOOL_TIMEOUT`, sinon un défaut que le SDK qualifie d'« effectively unbounded » | `dist/acp-agent.js:5864-5872` ; `sdk.d.ts:516-519` et le champ `timeout` de `McpHttpServerConfig` |
| `@agentclientprotocol/codex-acp` **1.12.0** | `url`, `http_headers` — aucun `tool_timeout_sec` | celui de Codex : `tool_timeout_sec` du serveur, sinon `DEFAULT_TOOL_TIMEOUT` = **300 s** | `dist/index.js:28766-28783` ; dépendance `@openai/codex` `^0.154.0`, qui résout à **0.154.0** ; `codex-rs/codex-mcp/src/rmcp_client.rs:103` et `connection_manager.rs:314-317` à l'étiquette `rust-v0.154.0` |

**Conséquence.** Avec Claude, l'appel attend la décision. Avec Codex, les
300 s de Codex et les cinq minutes de la demande coïncident : l'une ou l'autre
échéance l'emporte à quelques millisecondes près. Dans les deux cas rien ne
s'exécute, et la carte le dit — « Expired », ou « Withdrawn: the agent stopped
waiting » quand Codex raccroche le premier. Un accord donné dans la même
milliseconde s'exécute ; Codex a déjà dit à son modèle que l'appel avait
expiré, et une nouvelle tentative de sa part attend que la première soit
tranchée — les appels d'un agent passent un par un
(`crates/oxyn-ai/src/external/mcp/turn.rs`, « one call at a time ») —
puis redemande l'accord, à l'écran : jamais un rejeu silencieux
([I-13](../CLAUDE.md#i-13)).

**À refaire** à chaque montée de version de l'un des deux adaptateurs ou de
`@openai/codex` : un délai par défaut plus court que cinq minutes ferait
abandonner l'appel avant l'échéance de la demande.

### Environnement minimal d'un agent ACP — mesure du 2026-09-16

Oxyn lance l'agent avec une **liste blanche** d'environnement
([ADR-0030](adr/0030-outils-oxyn-exposes-a-un-agent-externe.md)). Reste à savoir
ce qu'il faut y mettre. Mesuré en envoyant un `initialize` seul à chaque
adaptateur — aucun prompt, aucune base touchée — et en faisant varier le seul
environnement. Seuls le `kind` et le `label` de l'état d'authentification sont
relevés, jamais une valeur d'environnement.

| Environnement de l'enfant | Claude Agent 0.78.0 | Codex 1.12.0 |
|---|---|---|
| complet | `account / Claude Max` | `account / ChatGPT` |
| `PATH`, `HOME`, `LANG`, `TMPDIR` | **`none / Not logged in`** | `account / ChatGPT` |
| + `LOGNAME` | `none / Not logged in` | — |
| + `SHELL` | `none / Not logged in` | — |
| + `SECURITYSESSIONID` | `none / Not logged in` | — |
| + **`USER`** | **`account / Claude Max`** | inchangé |
| `PATH`, `HOME`, `USER` | `account / Claude Max` | — |

**Conclusion : `USER`, et elle seule.** Aucun substitut ne convient, et Codex
n'en a pas besoin — la mesure ne se généralise pas d'un adaptateur à l'autre.

**Le piège, qui vaut plus que la valeur elle-même.** Sans `USER`, `initialize`
**réussit quand même**, `authMethods` vaut `[]` dans les deux cas, et
l'adaptateur note sur `stderr` : `[authStatus] session account carries no
identity signal; keeping`. Le refus n'arrive qu'au **prompt**. Un contrôle
arrêté au handshake ne voit donc rien, et l'utilisateur reçoit « connecte-toi »
après avoir posé une question, alors que `claude auth status` répond
`loggedIn: true` sur la même machine.

C'est le coût d'une liste blanche : **une variable manquante ne casse pas
l'agent, elle le dégrade en silence**. D'où la règle : chaque nom de la liste
porte la mesure qui l'y a mis.

### Modes et options de session — vérification du 2026-09-16

Un agent peut déclarer ses modes deux fois : par `modes` et par une option de
`configOptions` de catégorie `mode`. Le panneau affichait alors deux sélecteurs.

**Le schéma 1.7.0 ne tranche pas.** `agent-client-protocol-schema` 1.7.0 déclare
les deux champs côte à côte (`src/v1/agent.rs:877-884`) et ne parle ni de
remplacement ni d'obsolescence. Sa seule consigne sur les catégories
(`src/v1/agent.rs:2281-2286`) : elles servent l'ergonomie et « MUST NOT be
required for correctness ».

**La documentation du protocole tranche**, lue sur la source Markdown du site :

| Fichier | Ligne | Texte |
|---|---|---|
| [`protocol/session-config-options.md`](https://agentclientprotocol.com/protocol/session-config-options) | 333 | « Session Config Options supersede the older Session Modes API. » |
| idem | 340 | « Clients that support config options **SHOULD** use `configOptions` exclusively and ignore `modes` » |
| idem | 342 | « Agents **SHOULD** keep both in sync » |
| [`protocol/session-modes.md`](https://agentclientprotocol.com/protocol/session-modes) | 10-11 | « Dedicated session mode methods will be removed in a future version » |

**Ce qu'Oxyn en fait** (`crates/oxyn-ai/src/external/settings.rs`,
`supersede_modes`) : dès qu'une option de catégorie `mode` existe, `modes` et
le mode courant ne sont plus retenus, ni affichés, ni acceptés en changement. Un
agent qui ne déclare que `modes` garde son sélecteur : la consigne vise la
transition, pas les agents anciens.

### Confinement des adaptateurs ACP — mesure du 2026-09-23

Fonde [ADR-0032](adr/0032-agent-externe-confine-au-lancement.md). Mesuré avec
les adaptateurs épinglés (`@agentclientprotocol/claude-agent-acp` 0.78.0, qui
embarque `@anthropic-ai/claude-agent-sdk` 0.3.270 ; `@agentclientprotocol/codex-acp`
1.12.0, qui embarque `@openai/codex` 0.154.0). Le client de mesure refuse
toute demande d'autorisation, comme `permission_for` ; la consigne demande à
l'agent de lancer `touch` sur un fichier témoin.

**Un agent ne demande que ce que son mode lui fait demander.**

| Agent, mode | Demande au client ? | Fichier créé |
|---|---|---|
| Claude Agent, `auto` (mode initial, lu dans les réglages de l'utilisateur) | non | oui |
| Claude Agent, `default` (« Manual ») | oui, refus respecté | non |
| Codex, `agent` (mode initial) | non | oui |
| Codex, `read-only`, fichier dans `/tmp` | non | oui |
| Codex, `read-only`, fichier dans le répertoire personnel | oui, refus respecté | non |

Le mode d'un agent décrit ce qu'il décide **sans** demander ; le refus par défaut
de `permission_for` ne protège que ce qui est demandé.

**Claude Agent : les options du SDK passent par `_meta`.** Dans
`dist/acp-agent.js` (0.78.0), `newSession` étale
`params._meta.claudeCode.options` dans les options du SDK. Relu dans
`sdk.d.ts` (0.3.270) :

| Option | Texte de `sdk.d.ts` |
|---|---|
| `tools` | `string[] \| { preset }` ; `[]` retire tous les outils intégrés |
| `allowedTools` | « List of tool names that are auto-allowed without prompting for permission » ; une entrée `mcp__<serveur>` vaut pour tout le serveur |
| `strictMcpConfig` | ne garde que les serveurs MCP fournis par l'appelant |
| `settingSources` | les sources de réglages chargées ; `[]` n'en charge aucune |
| `allowDangerouslySkipPermissions` | requis par `bypassPermissions` ; l'adaptateur retire ce mode du catalogue à `false` |

Mesuré avec `tools: []`, `allowedTools: ["mcp__oxyn"]`, `strictMcpConfig: true`,
`settingSources: []`, `allowDangerouslySkipPermissions: false`, puis
`session/set_mode` à `default` : aucun appel d'outil sur la consigne `touch`,
`bypassPermissions` absent des modes, et l'outil d'un serveur MCP `oxyn` appelé
sans demande d'autorisation.

**Codex : `CODEX_CONFIG` et `INITIAL_AGENT_MODE`**, variables documentées par
le README de `codex-acp` 1.12.0 (« JSON object merged into the Codex session
config » ; « initial mode id: `read-only`, `agent`, or `agent-full-access` »).
Clés relues dans la [référence de configuration Codex](https://learn.chatgpt.com/docs/config-file/config-reference)
le 2026-09-23 :

| Clé | Texte de la référence |
|---|---|
| `features.shell_tool` | « Enable the default `shell` tool for running commands (stable; on by default) » |
| `features.unified_exec` | « Use the unified PTY-backed exec tool » |
| `features.hooks` | « Enable lifecycle hooks loaded from hooks.json or inline [hooks] config » |
| `features.apps` | « Enable app (connector) integrations (stable; on by default) » |
| `web_search` | `disabled \| cached \| indexed \| live` ; « `"disabled"` to remove the tool » |
| `mcp_servers.<id>.url`, `.bearer_token_env_var` | point d'accès HTTP, et variable d'environnement portant le jeton |
| `mcp_servers.<id>.default_tools_approval_mode` | `auto \| prompt \| writes \| approve` |
| `mcp_servers.<id>.enabled`, `plugins.<id>.enabled` | désactivation **une par une** ; « No single master key exists » |

Mesuré avec le shell, `unified_exec`, les hooks, les apps et la recherche web
coupés, en `read-only` :

* la consigne `touch` n'exécute plus rien ; Codex tente une édition de fichier,
  qui demande et est refusée ;
* un serveur MCP déclaré **par ACP** (`session/new`) demande l'autorisation à
  chaque appel, sous le genre `execute` et sans titre ni nom d'outil —
  `default_tools_approval_mode` passé par `CODEX_CONFIG` ne s'y applique pas ;
* le même serveur déclaré **dans `CODEX_CONFIG`** avec
  `default_tools_approval_mode = "approve"` est appelé sans demande.

**L'écart qui reste.** Codex charge encore les serveurs MCP et les plugins de
`~/.codex/config.toml` de l'utilisateur ; la mesure l'a vu tenter l'outil d'un
plugin de l'utilisateur. Aucune clé documentée ne les coupe en bloc, et la
référence ne documente aucun moyen d'ignorer ce fichier.

#### Les couches de configuration que Codex charge — relu le 2026-09-23

Relu dans le code source au tag `rust-v0.154.0` d'openai/codex (commit
`6b9826e3aa83b1a5947db50f4332cb9c65f1b340`), qui correspond à `@openai/codex`
0.154.0. Le registre npm ne publie aucune 0.154.x plus récente à cette date,
alors que `codex-acp` 1.12.0 en accepte une (`"@openai/codex": "^0.154.0"`).
Côté adaptateur, relu dans `dist/index.js` de `@agentclientprotocol/codex-acp`
1.12.0, tel que publié au registre (`gitHead`
`a7afd2ae077d625710194d9701b83595494449de`). Les numéros de ligne renvoient à
ces deux versions.

**Le chemin de lancement.** `codex-acp` démarre `codex app-server` sans autre
argument, avec son propre environnement et son propre répertoire de travail
(`startCodexConnection`, l. 22098-22106). La CLI démarre l'app-server avec
`LoaderOverrides::default()` : **aucun profil**, même si `--profile` est passé
([`cli/src/main.rs` l. 1353-1356](https://github.com/openai/codex/blob/rust-v0.154.0/codex-rs/cli/src/main.rs#L1353-L1356)).
À chaque `session/new`, `codex-acp` envoie `thread/start` avec deux champs
(l. 28596-28602) : `cwd`, qui reprend le `cwd` ACP, et `config`, qui contient
`CODEX_CONFIG` plus `projects.<cwd>.trust_level = "trusted"` (l. 28703-28708).
L'app-server verse ce `config` dans les surcharges de ligne de commande
([`app-server/src/config_manager.rs` l. 232-243](https://github.com/openai/codex/blob/rust-v0.154.0/codex-rs/app-server/src/config_manager.rs#L232-L243)).
`CODEX_CONFIG` forme donc la couche **`SessionFlags`**, au même rang que `-c`.
Le chargeur de configuration par thread de l'app-server est
`NoopThreadConfigLoader`
([`app-server/src/lib.rs` l. 510](https://github.com/openai/codex/blob/rust-v0.154.0/codex-rs/app-server/src/lib.rs#L510)).

**`CODEX_HOME`**
([`utils/home-dir/src/lib.rs` l. 13-62](https://github.com/openai/codex/blob/rust-v0.154.0/codex-rs/utils/home-dir/src/lib.rs#L13-L62)) :

* une valeur **vide** vaut une variable absente ;
* une valeur non vide doit désigner un répertoire existant. Sinon, Codex
  s'arrête en erreur. Le chemin est canonicalisé, et un chemin relatif se
  résout donc depuis le répertoire de travail **de Codex** ;
* en l'absence de `CODEX_HOME`, Codex prend `home_dir()` suivi de `.codex`,
  sans vérifier que ce répertoire existe.

**Les couches, de la plus faible à la plus forte.** L'ordre vient du
[commentaire de `load_config_layers_state`](https://github.com/openai/codex/blob/rust-v0.154.0/codex-rs/config/src/loader/mod.rs#L110-L124)
et du corps de cette fonction (l. 132-500).

| Rang | Couche | Source | Peut déclarer `mcp_servers` / `plugins` |
|---|---|---|---|
| 1 | valeurs par défaut du paquet | `config/defaults.toml`, embarqué dans le binaire | non : 17 lignes, aucune de ces deux tables |
| 2 | système | `/etc/codex/config.toml` sous Unix ([l. 66](https://github.com/openai/codex/blob/rust-v0.154.0/codex-rs/config/src/loader/mod.rs#L66)) | oui |
| 3 | cloud géré par l'entreprise | fragments TOML livrés par le serveur pour l'espace de travail connecté ([`cloud_config_layers.rs`](https://github.com/openai/codex/blob/rust-v0.154.0/codex-rs/config/src/cloud_config_layers.rs)) | oui |
| 4 | utilisateur | `$CODEX_HOME/config.toml` | oui |
| 5 | profil v2 | `$CODEX_HOME/<nom>.config.toml`, choisi par `--profile` | non active ici, puisque l'app-server ignore le profil. Un profil hérité, `[profiles.<nom>]`, ne porte aucune de ces deux tables ([`profile_toml.rs`](https://github.com/openai/codex/blob/rust-v0.154.0/codex-rs/config/src/profile_toml.rs)) |
| 6 | projet | tout `.codex/config.toml` trouvé entre la racine du projet et le `cwd` du thread. Le plus proche du `cwd` l'emporte | oui : `mcp_servers` et `plugins` ne figurent pas dans `PROJECT_LOCAL_CONFIG_DENYLIST` ([l. 75-88](https://github.com/openai/codex/blob/rust-v0.154.0/codex-rs/config/src/loader/mod.rs#L75-L88)) |
| 7 | `SessionFlags` | `-c`, `--config`, et le `config` de `thread/start`, c'est-à-dire `CODEX_CONFIG` | c'est la couche qu'écrit Oxyn |
| 8 | géré, fichier hérité | `/etc/codex/managed_config.toml` sous Unix, **indépendamment de `CODEX_HOME`** ([`layer_io.rs` l. 22, 222-233](https://github.com/openai/codex/blob/rust-v0.154.0/codex-rs/config/src/loader/layer_io.rs#L222-L233)) | oui |
| 9 | géré par MDM (macOS) | préférence gérée `config_toml_base64` du domaine `com.openai.codex` ([`macos.rs` l. 20-22](https://github.com/openai/codex/blob/rust-v0.154.0/codex-rs/config/src/loader/macos.rs#L20-L22)) | oui |

La documentation publique confirme les rangs 1 à 7 : la section
« Configuration precedence » de
[config-basic](https://learn.chatgpt.com/docs/config-file/config-basic),
relue le 2026-09-23. Elle confirme aussi les rangs 8 et 9, dans cet ordre, au-dessus de
`config.toml` : la page
[managed-configuration](https://learn.chatgpt.com/docs/enterprise/managed-configuration),
relue le même jour. `requirements.toml`, dans `/etc/codex/` ou dans la
préférence `requirements_toml_base64`, contraint les valeurs mais ne forme pas
une couche de configuration.

**La fusion est récursive, table par table**
([`merge_toml_values`, `merge.rs` l. 58](https://github.com/openai/codex/blob/rust-v0.154.0/codex-rs/config/src/merge.rs#L58)).
`mcp_servers` et `plugins` forment donc l'**union** des entrées de toutes les
couches. Une couche plus forte ne remplace que les clés qu'elle écrit. Deux
conséquences :

* `mcp_servers.<id>.enabled = false` au rang 7 coupe un serveur déclaré aux
  rangs 2 à 6 ;
* aux rangs 8 et 9, ce même réglage n'est écrasé que si la couche gérée écrit
  elle-même `enabled`.

**La couche de projet**
(`find_project_root`, [l. 1548-1583](https://github.com/openai/codex/blob/rust-v0.154.0/codex-rs/config/src/loader/mod.rs#L1548-L1583) ;
`discover_project_layers`, [l. 1696-1718](https://github.com/openai/codex/blob/rust-v0.154.0/codex-rs/config/src/loader/mod.rs#L1696-L1718)) :

* Elle part du **`cwd` du thread**, pas de celui du processus. Oxyn donne le
  même répertoire aux deux : le répertoire privé que crée `private_directory`,
  à la fois répertoire de travail du processus et `cwd` de `session/new`.
* La racine du projet est le premier ancêtre qui porte un marqueur de
  `project_root_markers`. Par défaut, ce marqueur est `.git` : soit un fichier,
  soit un répertoire qui contient `HEAD`. Sans marqueur trouvé, la racine est
  le `cwd` lui-même. Une liste vide désactive la remontée.
* Codex lit tous les `.codex/config.toml` de la racine au `cwd`, en sautant
  celui qui est `CODEX_HOME`.
* Une couche **non approuvée** est chargée mais désactivée. L'approbation se
  cherche d'abord sur la clé exacte du répertoire dans `projects`, puis sur la
  racine du projet, puis sur la racine du dépôt git (l. 1062-1105).
  L'approbation que `codex-acp` ajoute ne vise que le `cwd`.

**Ce que voit un lancement par Oxyn.** Le répertoire privé est neuf et vide,
et son nom est imprévisible. Il est créé dans le `TMPDIR` d'Oxyn : sous macOS,
`/var/folders/…/T/`, sans `.git` au-dessus par défaut. La racine du projet est
alors le répertoire privé lui-même, et aucune couche de projet n'existe. La
remontée ne s'ouvre que si un ancêtre de `TMPDIR` porte un `.git`. Par exemple,
un `TMPDIR` placé sous un répertoire personnel versionné. Il faut aussi que
l'utilisateur ait approuvé cet ancêtre ou la racine du dépôt.

**Ce qu'Oxyn en fait.** Il lit les rangs 2, 4 et 8, et coupe par leur nom les
serveurs et les plugins qu'ils déclarent. Il ne présente pas comme confiné un
Codex dont le rang 8 écrit `enabled = true`. Il ne lit ni le rang 3 ni le
rang 9 : [ADR-0033](adr/0033-couches-de-configuration-codex.md) décide ces
trois points.

### Ce que les adaptateurs ACP disent d'un appel MCP — relu le 2026-09-24

Question : comment reconnaître, dans les `tool_call` qu'un adaptateur diffuse,
un appel à un outil du serveur MCP d'Oxyn, pour ne pas le dessiner une seconde
fois à côté de sa carte ([UX-SPEC](UX-SPEC.md#ce-que-le-panneau-montre-dun-agent-externe)) ?
Relu dans les archives du registre npm (`npm pack`) le 2026-09-24.

**`@agentclientprotocol/claude-agent-acp` 0.78.0.**

* `dist/tools.js`, `toolInfoFromToolUse`, branche `default` (l. 335-340) :
  un outil que l'adaptateur ne connaît pas, donc tout outil MCP, a pour
  `title` son nom et pour `kind` `"other"`.
* `dist/acp-agent.js`, `toolCallNotification` (l. 7186-7210) : le `tool_call`
  porte aussi `name` (champ instable, derrière la feature
  `unstable_tool_call_name` de `agent-client-protocol-schema` 1.7.0 et non
  activée ici). Il porte surtout `_meta.claudeCode.toolName`, rempli par
  `claudeCodeMetaFromToolUse` (l. 7080-7098) avec le nom programmatique de
  l'outil. Les mises à jour de fin d'appel reprennent ce `_meta`.
* Pour un outil MCP, ce nom est `mcp__<serveur>__<outil>`, donc
  `mcp__oxyn__describe_schema`.

**`@agentclientprotocol/codex-acp` 1.12.0**, `dist/index.js` :

* `createMcpToolCallUpdate` (l. 23035-23045) et `createExecuteToolCallUpdate`
  (l. 23133-23143) : le `tool_call` a pour `kind` `"execute"`, pas `"other"`.
  Son `title` vaut `mcp.<serveur>.<outil>`. Son `rawInput` vaut
  `{server, tool, arguments}`, et son `_meta` `{is_mcp_tool_call: true}`.
* `completeItemEvent`, cas `mcpToolCall` (l. 25123-25130) : la mise à jour de
  fin porte le statut et le même `rawInput`, **sans `_meta`**.
* `createMcpToolProgressEvent` (l. 25289-25299) : les progressions ne portent
  pas de statut.

**Ce qu'Oxyn en fait.** Le signal retenu diffère selon l'adaptateur. Chez
Claude, c'est `_meta.claudeCode.toolName`. Chez Codex, c'est
`_meta.is_mcp_tool_call`, avec `rawInput.server` et `rawInput.tool`. Dans les
deux cas, le nom doit correspondre exactement à un outil que le serveur a
annoncé.

Le titre n'est pas retenu. Chez Claude, celui d'une commande shell est la
commande elle-même, qui peut s'écrire comme un nom d'outil d'Oxyn. `rawInput`
seul n'est pas retenu non plus : pour tout autre outil, ce sont les arguments
que le modèle écrit.

L'identifiant d'un appel reconnu est gardé jusqu'à sa fin : la fin chez Codex
ne se désigne pas elle-même.

## Relecture locale des résultats — 2026-09-10

La version de Tokio déjà résolue, `1.53.1`, reste inchangée. `oxyn-exec` active
sa fonctionnalité `rt` pour remettre le décodage de page au pool bloquant du
runtime de l'application. La documentation officielle confirme que
[`Handle`](https://docs.rs/tokio/1.53.1/tokio/runtime/struct.Handle.html) est
disponible sous `rt`, que `try_current` rend une erreur en l'absence de runtime
et que `spawn_blocking` utilise un exécuteur dédié aux opérations bloquantes.
Consultation du 2026-09-10 ; aucune nouvelle dépendance ni montée de version.
Le partage du budget et les limites de cette relecture sont décidés dans
[ADR-0012](adr/0012-lecture-pages-resultats.md).

## Fermeture GPUI et préférences — 2026-09-10

[`App::shutdown`](https://docs.rs/gpui/0.2.2/gpui/struct.App.html#method.shutdown)
de GPUI `0.2.2` accorde 100 ms aux handlers `on_app_quit`, selon sa documentation
officielle consultée le 2026-09-10 et le code résolu localement. L'attente de
sauvegarde de la dernière fenêtre est donc placée avant l'appel à `quit`, en
plus du hook. Cela ne prouve pas tous les chemins d'arrêt natifs ; leur recette
reste suivie dans IMPLEMENTATION-PLAN. Aucune version de dépendance n'a changé.

## Annulation des recherches locales — 2026-09-10

`oxyn-store` active `hooks` sur la version de `rusqlite` déjà résolue, `0.37.0`.
Aucune version ni entrée de verrouillage ne change. Le code source installé
`rusqlite-0.37.0/src/hooks/mod.rs` expose `Connection::progress_handler` et
sa désactivation par `progress_handler(0, None::<fn() -> bool>)` ; son manifeste
confirme la fonctionnalité `hooks`. Vérification locale du 2026-09-10 ; la
consultation de docs.rs a échoué dans l'outil de navigation.

Le handler est installé sous le verrou de la connexion locale et retiré par
un garde de portée. Il observe le jeton de la seule opération active tous les
1 000 pas de la machine virtuelle SQLite. L'attente du verrou et le délai
SQLite de base occupée ne sont pas interrompus par ce handler ; une tâche
annulée en attente ne commence pas son opération après acquisition du verrou.
Une écriture déjà validée conserve son résultat de succès.

La vérification globale `verifier_versions.py` du 2026-09-10 signale aussi
Redis `1.7.0` au registre contre `1.6.0` dans le relevé historique du 2026-09-05.
Cet écart concerne une piste de driver ultérieure, pas la dépendance rusqlite
modifiée ici ; aucune montée de version n'est effectuée dans ce lot.


## Sessions SQLite en mémoire — 2026-09-10

La [documentation SQLite](https://www.sqlite.org/inmemorydb.html) distingue les
bases privées ouvertes sous `:memory:` des bases nommées en mémoire ouvertes
avec `mode=memory&cache=shared`. Ces dernières sont partagées par les connexions
d'un même processus utilisant le même nom et disparaissent à la fermeture de
la dernière connexion. Source consultée le 2026-09-10.

Le driver utilise maintenant un nom interne dérivé de l'identité de configuration
pour que les sessions d'un même profil retrouvent leur base en mémoire. Deux
profils restent isolés. Ce nom ne figure pas dans les diagnostics. La restriction
lecture seule est appliquée par `query_only`, relevée après ouverture, puis
conservée dans les limites de chaque requête ; un appelant demandant des limites
inscriptibles ne peut pas la lever. Les tests de contrat vérifient lecture
partagée, refus d'écriture, survie d'une session voisine et nouvelle base vide
après fermeture de la dernière session. Aucune dépendance ni version n'a changé.

### Introspection des contraintes — vérification du 2026-09-10

`pg_constraint` fournit `contype`, `conkey` et les noms ; les colonnes de clés
composées sont relues dans leur ordre. `pg_get_constraintdef` produit une
reconstruction par le moteur, pas le texte saisi à l'origine.
Sources : [catalogue des contraintes PostgreSQL](https://www.postgresql.org/docs/current/catalog-pg-constraint.html)
et [fonctions d'information](https://www.postgresql.org/docs/current/functions-info.html).
Les attributs NOT NULL sans entrée correspondante sont lus depuis
`pg_attribute.attnotnull`, sans leur inventer de nom. Les limites 1024 entrées
et 16 Kio par définition sont des limites produit, avec erreur explicite.

Le test de compatibilité des anciens caches ajoute seulement
`serde_json.workspace = true` aux dépendances de test d'`oxyn-catalog`.
La version de workspace **1.0.151** est conservée, vérifiée avec
`cargo info serde_json@1.0.151` ; [registre](https://crates.io/crates/serde_json/1.0.151).
Aucune nouvelle version n'est introduite.

### Contraintes SQLite — vérification du 2026-09-10

Les contraintes PK, UNIQUE, CHECK, NOT NULL et REFERENCES sont déclarées dans
le SQL stocké par SQLite. Les PRAGMA ne suffisent pas à restituer leurs noms et
leurs clauses CHECK. L'extraction du driver conserve les tranches originales
de `sqlite_schema.sql` ; elle ne reconstruit pas un CREATE TABLE depuis l'AST.
Sources : [CREATE TABLE SQLite](https://www.sqlite.org/lang_createtable.html)
et [table du schéma](https://www.sqlite.org/schematab.html).
Les contraintes de table adjacentes sans virgule, les noms entre guillemets,
les commentaires et les clauses `ON CONFLICT` sont vérifiés contre le moteur
SQLite embarqué par les tests du driver. La borne de 1 Mio de SQL source,
les 1024 contraintes et les 16 Kio par clause sont des limites produit ; tout
dépassement est signalé. Aucune nouvelle dépendance n'est ajoutée.

Le statut de la grille Constraints (`229:7998`) repose sur
`pg_constraint.convalidated`, vérifié dans la documentation PostgreSQL citée
ci-dessus. Un CHECK `NOT VALID` et son passage à `VALIDATE CONSTRAINT` sont
couverts par un test sur base jetable. SQLite conserve un statut inconnu, y
compris lorsque des données ne respectant pas un CHECK ont été insérées avec
`ignore_check_constraints` activé ; un test empêche de présenter la seule
présence de la clause comme une preuve de validation.

### Relations entrantes — vérification du 2026-09-10

Les fonctions de PRAGMA SQLite sont utilisables dans SELECT et acceptent le
schéma comme dernier argument : [documentation](https://www.sqlite.org/pragma.html#pragma_functions).
Les références sans liste de colonnes utilisent la clé primaire de la cible ;
leur comparaison utilise l'affinité et la collation des colonnes parentes :
[clés étrangères SQLite](https://www.sqlite.org/foreignkeys.html).
Le driver lit les métadonnées natives via `Connection::column_metadata`, API
vérifiée dans le source installé de rusqlite, sans ajout de dépendance.

PostgreSQL expose la cible dans `pg_constraint.confrelid`, les listes de
colonnes dans `conkey`/`confkey` et les index dans `pg_index`.
Les colonnes INCLUDE sont exclues du test d'unicité en utilisant `indnkeyatts` ;
les index invalides, partiels, d'expression ou de comparaison non établie ne
permettent pas de certifier une relation un-à-un.
Sources : [pg_constraint](https://www.postgresql.org/docs/current/catalog-pg-constraint.html)
et [pg_index](https://www.postgresql.org/docs/current/catalog-pg-index.html).
Les limites 1024 clés, 128 colonnes par clé, 16 Kio par texte et 16 Mio de textes
SQLite parcourus sont des limites produit, pas des limites attribuées aux SGBD.

### Définitions DDL — vérification du 2026-09-10

SQLite conserve les statements dans
[sqlite_schema](https://www.sqlite.org/schematab.html). Le lecteur prend objet,
index et triggers dans un même curseur ; la qualification ne modifie que leurs
noms de déclaration. Un test les recrée dans un espace attaché distinct et
vérifie le fonctionnement du trigger et l'absence d'objet créé dans `main`.

PostgreSQL expose des fonctions de reconstruction (`pg_get_viewdef`,
`pg_get_indexdef`, `pg_get_triggerdef`, `pg_get_ruledef`, `pg_get_constraintdef`)
dans ses [fonctions d'information](https://www.postgresql.org/docs/current/functions-info.html).
Les paramètres des séquences proviennent de
[pg_sequence](https://www.postgresql.org/docs/current/catalog-pg-sequence.html).
La forme des colonnes générées/identity et des tables partitionnées suit
[CREATE TABLE](https://www.postgresql.org/docs/current/sql-createtable.html).
Le lecteur utilise `attgenerated`, présent dans
[pg_attribute de PostgreSQL 12](https://www.postgresql.org/docs/12/catalog-pg-attribute.html) :
`OBJECT_DEFINITION` n'est annoncé qu'à partir de cette version reconnue, et pas
pour Redshift. Les versions plus anciennes ou illisibles n'obtiennent pas
cette capacité par supposition.

Les politiques RLS sont relues depuis
[pg_policy](https://www.postgresql.org/docs/17/catalog-pg-policy.html), avec les
noms de rôles de la vue publique `pg_roles` (OID 0 signifie PUBLIC). Les états
[ENABLE/FORCE ROW LEVEL SECURITY](https://www.postgresql.org/docs/17/sql-altertable.html)
sont reconstruits. Les tests sur PostgreSQL 17.11 vérifient leur maintien
après recréation, ainsi que identity/serial, colonnes générées, index, triggers,
vues, séquences et racines partitionnées. Les scripts ne sont appliqués que
par ces fixtures explicitement isolées, via `AssertSqlSafe` de sqlx après audit.
Aucun trajet produit n'applique automatiquement la définition affichée.

### Partitions PostgreSQL et instruction courante — vérification du 2026-09-10

La création d'un enfant utilise
[PARTITION OF](https://www.postgresql.org/docs/17/sql-createtable.html), le lien
direct de [pg_inherits](https://www.postgresql.org/docs/17/catalog-pg-inherits.html)
et la borne de [pg_class](https://www.postgresql.org/docs/17/catalog-pg-class.html).
Le DDL conserve les options locales et laisse PostgreSQL cloner les éléments
hérités. Les tests de recréation vérifient parent, borne, défaut et sous-partition.

L'instruction courante est résolue dans `oxyn-query`, sans dépendance native
ni I/O. Le scanner conserve les corps SQLite/PG ; les positions UTF-8 invalides,
textes incomplets et identifiants SQLite rendant la frontière ambiguë produisent
une demande de sélection explicite. Cette résolution ne remplace pas le
classificateur du bus et n'ajoute aucune nouvelle version de dépendance.

La compatibilité déclarée dès PostgreSQL 12 tient compte de l'absence de
`inhdetachpending` dans [pg_inherits 12](https://www.postgresql.org/docs/12/catalog-pg-inherits.html)
et de `tgparentid` dans [pg_trigger 12](https://www.postgresql.org/docs/12/catalog-pg-trigger.html).
Ces champs sont lus à travers JSONB, sans référence SQL directe qui casserait
la lecture des tables ordinaires. Sans provenance native des triggers, la
définition d'un enfant en portant est refusée plutôt qu'inférée depuis des
noms ou expressions similaires. Les essais réels de ce lot utilisent PostgreSQL
17.11 ; un serveur PostgreSQL 12 n'a pas été exécuté.

## Fin d'un commentaire `--` — vérification du 2026-09-16

Le découpeur d'`oxyn-query` décide quel texte est une instruction, et le
classificateur en déduit qu'elle écrit. Il doit terminer un commentaire de ligne
**là où le serveur le termine**. Aucun des deux choix n'est sûr par défaut.

- **Finir trop tard** cache une instruction que le serveur exécute.
- **Finir trop tôt** fait lire comme du code un `/*` ou une apostrophe que le
  serveur lit comme commentaire. L'instruction qui suit disparaît alors dans un
  faux commentaire de bloc.

| Moteur | Fin de `--` | Source |
|---|---|---|
| PostgreSQL, serveur | `\n` **ou** `\r` | [`scan.l` au tag `REL_17_6`](https://github.com/postgres/postgres/blob/REL_17_6/src/backend/parser/scan.l#L224-L227), l. 224-227 : `newline [\n\r]`, `comment ("--"{non_newline}*)`. Même définition sur `master` au commit `a4f18fd8f280`, l. 206-209 |
| PostgreSQL, `psql` | `\n` ou `\r` | [`psqlscan.l` au tag `REL_17_6`](https://github.com/postgres/postgres/blob/REL_17_6/src/fe_utils/psqlscan.l#L160-L163), l. 160-163 |
| SQLite | `\n` ou NUL, **pas** `\r` | [`tokenize.c`](https://github.com/sqlite/sqlite/blob/version-3.50.2/src/tokenize.c), `sqlite3GetToken`, `case CC_MINUS` : `for(i=2; (c=z[i])!=0 && c!='\n'; i++){}`. Identique dans l'amalgamation **3.50.2** qu'embarque `libsqlite3-sys` `0.35.0` (`sqlite3.c`, l. 181599) |
| `sqlparser` `0.62.0` | `\n` ; aussi `\r` pour `PostgreSqlDialect` seulement | `src/tokenizer.rs`, `tokenize_single_line_comment`, l. 2039-2044 |

Les commentaires `/* */` ne donnent aucun rôle à `\r`. PostgreSQL les imbrique,
SQLite non.

**Vérifié à l'exécution.** Un PostgreSQL 17.11 jetable a été interrogé en
protocole simple et en protocole étendu Parse/Bind/Execute/Sync, celui qu'emploie
`sqlx` au `prepare`.

- `-- x\rDROP TABLE audit` supprime la table dans les deux protocoles.
- `SELECT 1; -- x\rDROP TABLE audit` la supprime en protocole simple. En
  protocole étendu, le serveur répond `42601 cannot insert multiple commands into
  a prepared statement` et n'exécute rien.
- Un `;` vide en tête ne compte pas comme une commande :
  `; -- x\rDROP TABLE audit` passe en protocole étendu.

SQLite 3.53.3 a été interrogé via `executescript` : le texte qui suit un `\r`
isolé reste un commentaire.

**Ce qu'Oxyn en fait.** `LineCommentEnd`, dans `crates/oxyn-query/src/split.rs`,
fixe la fin de commentaire par dialecte :

- `Postgres` : `\r` ou `\n` ;
- `Sqlite` : `\n` ;
- tout dialecte **non vérifié** ici, Redshift compris (son lexer est fermé) : un
  commentaire de ligne qui contient un `\r` isolé suivi de texte rend son
  instruction illisible, donc `Unknown`.

Non vérifié :

- MySQL, SQL Server, ClickHouse, DuckDB, Snowflake, BigQuery, Oracle ;
- le traitement d'un NUL dans le message Parse de PostgreSQL.

## Fournisseur Anthropic — vérification du 2026-09-16

Relevé pour l'implémentation de `crates/oxyn-llm/src/anthropic/`. **La
documentation a changé de domaine** : `docs.anthropic.com` et `docs.claude.com`
répondent `301`/`302` vers `platform.claude.com/docs/en/…`. Les URL ci-dessous
sont celles qui répondent directement.

### Transport

| Fait | Valeur | Source | Vérifié le |
|---|---|---|---|
| Génération | `POST /v1/messages` | [Messages API](https://platform.claude.com/docs/en/api/messages) | 2026-09-16 |
| Comptage de jetons | `POST /v1/messages/count_tokens`, réponse `{"input_tokens": N}` | [Token counting](https://platform.claude.com/docs/en/build-with-claude/token-counting) | 2026-09-16 |
| Liste des modèles | `GET /v1/models`, `limit` de 1 à 1000 (défaut 20), curseurs `after_id`/`before_id` | [List Models](https://platform.claude.com/docs/en/api/models/list) | 2026-09-16 |
| En-têtes obligatoires | `x-api-key`, `anthropic-version`, `content-type: application/json` | [Messages API](https://platform.claude.com/docs/en/api/messages) | 2026-09-16 |
| Valeur d'`anthropic-version` | `2023-06-01` | idem, et exemples cURL de toutes les pages consultées | 2026-09-16 |
| Champ obligatoire | `max_tokens` (avec `model` et `messages`) | idem | 2026-09-16 |
| Taille maximale d'une requête | 32 Mo sur Messages et sur le comptage | [Errors § request size limits](https://platform.claude.com/docs/en/api/errors) | 2026-09-16 |

> **`temperature`, `top_p` et `top_k` sont documentés comme dépréciés**, et les
> modèles récents n'acceptent plus que `1.0` (respectivement `≥ 0.99`). Oxyn ne
> les envoie que si l'appelant les a explicitement fixés — le comportement était
> déjà celui-là, et cette note dit pourquoi il ne faut pas le « corriger » en
> posant un défaut.

### Flux SSE

Huit types d'événements, chacun portant son nom en `event:` **et** un champ
`type` dans sa charge. Le décodeur lit le champ, pas le nom : les deux sont
redondants et la charge est ce qu'un mandataire altère le moins.

| Événement | Ce qu'il porte |
|---|---|
| `message_start` | l'objet `message`, `content` vide, `usage` d'entrée déjà renseigné |
| `content_block_start` | `index` et `content_block` (avec son `type`) |
| `content_block_delta` | `index` et `delta` |
| `content_block_stop` | `index` |
| `message_delta` | `delta.stop_reason`, `delta.stop_sequence`, et `usage` **cumulatif** |
| `message_stop` | rien |
| `ping` | rien — maintien de connexion, en nombre quelconque |
| `error` | `error.type` et `error.message`, après un `200` |

Quatre types de `delta`, tous vérifiés sur les exemples de la page
[Streaming](https://platform.claude.com/docs/en/build-with-claude/streaming) :
`text_delta` (`text`), `input_json_delta` (`partial_json`), `thinking_delta`
(`thinking`), `signature_delta` (`signature`). Le `signature_delta` arrive juste
avant le `content_block_stop` du bloc de raisonnement.

Types de blocs de contenu rencontrés : `text`, `tool_use`, `thinking`,
`redacted_thinking` (champ `data`), `server_tool_use`,
`web_search_tool_result`. Oxyn n'offre aucun outil côté serveur ; les deux
derniers sont suivis sans être exploités.

> **Il n'y a pas de sentinelle de fin.** Contrairement aux protocoles
> compatibles OpenAI, aucun `[DONE]` : la fin est `message_stop`. Une
> fermeture du flux sans `message_stop` — propre ou non — n'est pas une fin :
> Oxyn la classe `StopReason::Interrupted` et jette les blocs sans
> `content_block_stop`.

### Raisons d'arrêt

Sept valeurs, [Handling stop reasons](https://platform.claude.com/docs/en/build-with-claude/handling-stop-reasons),
vérifiées le 2026-09-16 : `end_turn`, `max_tokens`, `stop_sequence`, `tool_use`,
`pause_turn`, `refusal`, `model_context_window_exceeded`. La page de versionnage
annonce que cette liste peut grandir : une valeur inconnue est **conservée**
(`StopReason::Other`) et jamais rabattue sur `end_turn`.

### Erreurs et reprise

| Statut | `error.type` | Famille retenue |
|---|---|---|
| 400 | `invalid_request_error` | permanente |
| 401 | `authentication_error` | permanente |
| 402 | `billing_error` | permanente |
| 403 | `permission_error` | permanente |
| 404 | `not_found_error` | permanente |
| 409 | `conflict_error` | permanente |
| 413 | `request_too_large` | permanente |
| 429 | `rate_limit_error` | **transitoire** |
| 500 | `api_error` | **transitoire** |
| 504 | `timeout_error` | **ambiguë** |
| 529 | `overloaded_error` | **transitoire** |

Source : [Claude API errors](https://platform.claude.com/docs/en/api/errors).

#### Rejouer un `500`, `502` ou `504` — relevé le 2026-09-17

| Statut | Anthropic ([Claude API errors](https://platform.claude.com/docs/en/api/errors)) | OpenAI ([Error codes](https://developers.openai.com/api/docs/guides/error-codes)) | Famille retenue |
|---|---|---|---|
| `500` | `api_error` : « Retry the request with exponential backoff » | « Retry your request after a brief wait and contact us if the issue persists » | transitoire — **sourcée** chez les deux |
| `502` | non documenté | non documenté | transitoire — **non vérifié** : classement antérieur conservé faute de source |
| `504` | `timeout_error` : « The request timed out **while processing** » | non documenté | **ambiguë** |

Le `504` n'est pas une surcharge : le traitement avait commencé, et la réponse a
pu être produite — et facturée — sans arriver (I-13). Une passerelle placée
devant un fournisseur compatible répond de même `504` après avoir transmis la
requête. Il est donc ambigu, comme `LlmError::ResponseTimeout` et
`LlmError::ConnectionLost`, et se projette sur `OxynError::OutcomeUnknown`.

Aucune des deux pages ne dit si une requête en échec a été facturée. La page
Anthropic ajoute que ses SDK rejouent d'eux-mêmes « 5xx server errors » : c'est
un choix de SDK, pas une garantie d'absence d'effet, et Oxyn ne rejoue jamais de
lui-même.
Corps d'erreur : `{"type":"error","error":{"type":…,"message":…},"request_id":…}`.

> **`529` a fait corriger le code.** Il ne fait partie d'aucun standard HTTP, et
> la table de `LlmError::class` le rangeait dans « reste des `5xx` », donc
> permanent : l'interface aurait proposé « reconfigurer » pour une surcharge
> passagère. Il est désormais transitoire.

L'en-tête `retry-after` est renvoyé sur un `429` de limitation, **en secondes**
([Rate limits § response headers](https://platform.claude.com/docs/en/api/rate-limits)).
Deux exceptions documentées où il est absent : le `429` de plafond de dépense
mensuel, reconnaissable à `error.details.error_code = enforced_spend_limit_reached`,
et pour lequel rejouer échoue jusqu'au mois suivant. Oxyn ne rejoue jamais de
lui-même (I-13) : cet en-tête n'est donc pas encore lu, et le noter ici évite
qu'on le croie traité.

### Raisonnement : deux réglages distincts

C'est le point qui se rate, parce que les deux noms se ressemblent et ne font
pas la même chose.

| Réglage | Où il vit | Valeurs | Source |
|---|---|---|---|
| Effort | `output_config.effort`, premier niveau du corps | `low`, `medium`, `high`, `xhigh`, `max` | [Effort](https://platform.claude.com/docs/en/build-with-claude/effort) |
| Mode de réflexion | `thinking.type` | `adaptive`, `enabled` (avec `budget_tokens`), `disabled` | [Thinking](https://platform.claude.com/docs/en/build-with-claude/thinking) |
| Rendu du raisonnement | `thinking.display` | `summarized`, `omitted` (défaut sur plusieurs modèles), `updates` (bêta) | idem |

Deux pièges vérifiés, et ce sont eux qui dictent le code :

1. **`thinking.type: "adaptive"` est refusé par les modèles antérieurs** avec un
   `400` (`adaptive thinking is not supported on this model`), et
   `thinking.type: "enabled"` est refusé par les modèles récents, qui renvoient
   au couple `adaptive` + `output_config.effort`. Aucun mode n'est donc
   universel : Oxyn n'envoie `thinking` **que** si l'appelant demande un budget,
   et se contente de `output_config.effort` sinon.
2. **`display` vaut `omitted` par défaut** sur la plupart des modèles : le bloc
   de raisonnement revient alors avec un `thinking` vide mais **une signature**.
   Il doit quand même être conservé et renvoyé, sinon le tour suivant échoue.

Les blocs de raisonnement se renvoient **inchangés** : l'API vérifie leur
signature, et une modification produit un `400` dont le message est
`` `thinking` or `redacted_thinking` blocks in the latest assistant message
cannot be modified ``. Les `redacted_thinking` comptent, y compris ceux dont le
champ `thinking` est vide.

#### Quels modèles acceptent un effort, et d'où Oxyn le sait

**Le code ne contient aucune liste de modèles.** `ModelInfo::reasoning_efforts`
est rempli à l'exécution, niveau par niveau, depuis
`capabilities.effort.{low,medium,high,xhigh,max}.supported` de la réponse de
`GET /v1/models` — un seul appelant,
`crates/oxyn-llm/src/anthropic/wire.rs`. C'est ce qui fait qu'aucune valeur ne
peut périmer : le fournisseur déclare, Oxyn relaie, et une liste **vide**
signifie « non déclaré », jamais « aucun ».

La liste ci-dessous est une **référence croisée** pour savoir à quoi s'attendre
— elle n'alimente aucun chemin de code, et elle périmera. Relevée le
2026-09-16 sur la page [Effort](https://platform.claude.com/docs/en/build-with-claude/effort),
section « Supported models » :

> `claude-fable-5-1`, `claude-mythos-5-1`, `claude-fable-5`, `claude-mythos-5`,
> `claude-mythos-preview`, `claude-opus-5`, `claude-opus-4-8`,
> `claude-opus-4-7`, `claude-opus-4-6`, `claude-opus-4-5-20251101`,
> `claude-sonnet-5`, `claude-sonnet-4-6`.

Deux réserves de la même page, et elles comptent pour l'interface :
**`xhigh` n'est pas offert partout** (« Not every model that supports `max`
supports `xhigh` »), et le défaut est `high` — « setting `effort` to `"high"`
produces exactly the same behavior as omitting the `effort` parameter ». Un
sélecteur qui afficherait les cinq niveaux pour tout modèle produirait donc un
bouton `xhigh` qui échoue sur une partie du catalogue ; c'est exactement pour ça
que la liste vient de l'API et non d'ici.

Le point d'accès de liste déclare ces capacités par modèle :
`capabilities.thinking.supported`, `capabilities.thinking.types.{adaptive,enabled}.supported`,
et `capabilities.effort.{supported,low,medium,high,xhigh,max}.supported` — la
documentation nomme d'ailleurs ce dernier « Effort (reasoning\_effort) support ».
C'est ce qui alimente `ModelInfo::supports_reasoning` et
`ModelInfo::reasoning_efforts`.

### Cache de prompt

| Fait | Valeur | Source |
|---|---|---|
| Marqueur | `"cache_control": {"type": "ephemeral"}` | [Prompt caching](https://platform.claude.com/docs/en/build-with-claude/prompt-caching) |
| Durée | `ttl` `"5m"` (défaut) ou `"1h"`, **sans** en-tête bêta | idem |
| Emplacements | blocs de `system`, blocs de contenu de `messages`, **dernier** outil de `tools` | idem |
| Nombre maximal de marqueurs | **4** ; un cinquième fait échouer la requête | idem |
| Longueur minimale mise en cache | 512 à 4096 jetons selon le modèle ; en dessous, aucun cache et **aucune erreur** | idem |
| Consommation | `cache_creation_input_tokens` (écrit), `cache_read_input_tokens` (lu) | idem |

Le total d'entrée vaut `input_tokens + cache_creation_input_tokens +
cache_read_input_tokens` : `input_tokens` ne compte que ce qui suit le dernier
marqueur. C'est pourquoi `ChatEvent::Usage` les porte séparément plutôt que de
les additionner.

Oxyn pose au plus quatre marqueurs et s'arrête avant la limite plutôt que de
laisser arriver un `400` pour un réglage que l'utilisateur n'a pas conscience
d'avoir posé.

### Outils

`input_schema` et non `parameters`. Un résultat d'outil est un bloc
`{"type":"tool_result","tool_use_id":…,"content":…}` dans un message de rôle
`user`, avec `is_error: true` quand l'exécution a échoué. Le flux à granularité
fine s'active **par outil** avec `eager_input_streaming: true`, remplaçant
l'en-tête bêta `fine-grained-tool-streaming-2025-05-14`
([Fine-grained tool streaming](https://platform.claude.com/docs/en/agents-and-tools/tool-use/fine-grained-tool-streaming)) ;
la documentation avertit alors que le JSON accumulé **peut être invalide**.
Oxyn ne l'active pas, mais garde le parse protégé : un `max_tokens` atteint au
milieu d'un paramètre produit le même JSON tronqué, sans aucune option.

### Tarifs — délibérément absents

`GET /v1/models` ne publie **aucun** tarif, et aucune des pages consultées n'en
donne sous une forme lisible par un programme. `ModelInfo::cost` reste donc
`None` pour ce fournisseur. Écrire une grille en dur serait exactement la valeur
plausible et fausse qu'I-12 interdit ; OpenRouter, qui publie ses prix dans sa
réponse, reste le seul fournisseur dont le coût est renseigné (voir
[Fournisseur OpenRouter](#fournisseur-openrouter--vérification-du-2026-09-24)).

### Côté OpenAI, pour la parité

| Fait | Valeur | Source | Vérifié le |
|---|---|---|---|
| Effort de raisonnement | `reasoning_effort` au premier niveau (Chat Completions) ; `reasoning.effort` sur l'API Responses | [Reasoning](https://developers.openai.com/api/docs/guides/reasoning) | 2026-09-16 |
| Valeurs | dépendantes du modèle, parmi `none`, `minimal`, `low`, `medium`, `high`, `xhigh`, `max` | idem | 2026-09-16 |
| Jetons en cache | `usage.prompt_tokens_details.cached_tokens` | [Chat object](https://developers.openai.com/api/docs/api-reference/chat/object) | 2026-09-16 |
| Jetons de raisonnement | `usage.completion_tokens_details.reasoning_tokens` | idem | 2026-09-16 |
| Refus | `delta.refusal`, champ distinct de `delta.content` | idem | 2026-09-16 |
| Raisons d'arrêt | `stop`, `length`, `tool_calls`, `content_filter`, `function_call` (déprécié) | idem | 2026-09-16 |
| Fin de flux | `finish_reason` renseigné sur le dernier fragment de contenu ; puis, avec `stream_options.include_usage`, un fragment à `choices` vide portant `usage` ; puis `data: [DONE]`. « If the stream is interrupted, you may not receive the final usage chunk » | [Create chat completion](https://developers.openai.com/api/docs/api-reference/chat/create) | 2026-09-16 |

`ReasoningEffort` ne porte que les cinq niveaux communs aux deux fournisseurs.
`none` et `minimal` sont propres à OpenAI ; l'énumération est `#[non_exhaustive]`
et `ReasoningEffort::parse` rend `None` dessus plutôt que de les rabattre sur
`low`, ce qui changerait la demande de l'utilisateur.

### Fin de flux chez les serveurs locaux compatibles OpenAI

Oxyn classe `StopReason::Interrupted` un flux fermé sans `finish_reason` ni
`[DONE]`. Le risque mesuré : un serveur local qui n'enverrait jamais
`finish_reason` verrait toutes ses réponses marquées interrompues. Relevé **dans
les sources**, le 2026-09-16, sur les révisions nommées.

| Serveur | Révision | `finish_reason` final | `[DONE]` | Limite de contexte | Erreur en plein flux |
|---|---|---|---|---|---|
| Ollama | [`a43fad18`](https://github.com/ollama/ollama/tree/a43fad18b088095de20fbd7a8f0de50824cf5d27) (`main`, 2026-09-15) | toujours, sur une trame dédiée : `FinishChunk` (`openai/openai.go`) vaut `DoneReason`, `stop` par défaut, `tool_calls` si un appel a été émis | oui, après la trame finale et l'éventuelle trame `usage` (`ChatWriter.writeResponse`, `middleware/openai.go`) | `length` : le moteur est `llama-server`, dont l'arrêt `limit` devient `DoneReasonLength` (`llm/llama_server.go`) ; une génération ouverte est bornée à plusieurs fenêtres de contexte (`boundedNumPredict`) | **ni `finish_reason` ni `[DONE]`**, et le message est perdu : `streamResponse` (`server/routes.go`) écrit `{"error":…}` après un `200`, que `ChatWriter.Write` relit comme une réponse vide. Oxyn : `Interrupted` |
| llama.cpp | [`60199339`](https://github.com/ggml-org/llama.cpp/tree/60199339bcff9092dd7273d371b52308c85a92de) (`master`, 2026-09-16) | toujours, sur une trame dédiée : `to_json_oaicompat_chat_stream` (`tools/server/server-task.cpp`) vaut `stop` ou `tool_calls` sur fin de modèle ou mot d'arrêt, `length` sinon | oui, quand plus aucun résultat n'arrive (`tools/server/server-context.cpp`) | `length` : sans décalage de contexte, `STOP_TYPE_LIMIT` quand la fenêtre est pleine ; avec, la génération continue en décalant | `data: {"error":…}`, puis fermeture **sans** `[DONE]`. Oxyn : `ProviderError` |
| LM Studio | — | **non mesuré** : logiciel fermé | non mesuré | non mesuré | non mesuré |

Conclusion : la règle tient pour les deux serveurs ouverts. Un arrêt en limite
de contexte y est un `MaxTokens`, pas une coupure. llama.cpp envoie en outre des
commentaires SSE `:` de maintien de connexion, que le décodeur ignore. Les faits
> ci-dessus viennent de `developers.openai.com`, qui sert la même
> documentation — c'est aussi le domaine déjà utilisé plus haut dans ce fichier
> pour la fiche de `gpt-6-astra`.

## Fournisseur Gemini — vérification du 2026-09-24

| Fait | Valeur | Source | Vérifié le |
|---|---|---|---|
| Hôte | `https://generativelanguage.googleapis.com` | [Generate content](https://ai.google.dev/api/generate-content) | 2026-09-24 |
| Version de chemin | `v1beta` | idem | 2026-09-24 |
| Chemin du flux | `models/{modèle}:streamGenerateContent` | idem | 2026-09-24 |
| Paramètre de requête | `alt=sse` — sans lui l'API rend un tableau JSON entier plutôt qu'un flux | idem | 2026-09-24 |
| En-tête de clé | `x-goog-api-key` | [Versions d'API](https://ai.google.dev/gemini-api/docs/api-versions) | 2026-09-24 |

`v1beta` plutôt que `v1` : la référence de `generateContent` et de
`streamGenerateContent` ne documente le chemin que sous `/v1beta/…`. La page
des versions d'API confirme que `v1` est la version stable tandis que `v1beta`
« porte les fonctionnalités récentes » et reste le défaut des SDK officiels.
`GeminiProvider::with_api_version` permet de basculer vers `v1` le jour où
Google y fait migrer `streamGenerateContent`, sans toucher `GEMINI_BASE_URL`.

La clé passe par l'en-tête `x-goog-api-key` — montré dans les exemples `curl`
de la page des versions d'API — et non par le paramètre `?key=…` que la
référence de `generateContent` utilise elle aussi dans ses propres exemples :
un en-tête ne finit jamais dans une URL journalisée, un paramètre de requête
si ([I-03](../CLAUDE.md#i-03), [SECURITY](SECURITY.md#secrets)).

[ARCHITECTURE §7.5](ARCHITECTURE.md#75-abstraction-des-fournisseurs) établit
l'état réel du fournisseur : `GeminiProvider` construit cette requête puis
refuse l'appel (`LlmError::NotImplemented`) avant tout `.send()`. Le schéma
exact du flux de réponse et la liste des modèles disponibles restent non
vérifiés ; aucune valeur n'est écrite pour eux ici.

## Fournisseur OpenRouter — vérification du 2026-09-24

| Fait | Valeur | Source | Vérifié le |
|---|---|---|---|
| Devise des tarifs | `USD` | [Models](https://openrouter.ai/docs/guides/overview/models) | 2026-09-24 |
| Unité | prix par jeton, rendu en chaîne de caractères ; `WirePricing::into_cost` le convertit en prix par million | idem | 2026-09-24 |
| Champ `prompt` | coût par jeton d'entrée | idem | 2026-09-24 |
| Champ `completion` | coût par jeton de sortie | idem | 2026-09-24 |

La réponse de l'API ne porte jamais la devise — seule cette page de
documentation la donne. C'est pourquoi `OPENROUTER_CURRENCY` est une constante
injectée côté Oxyn plutôt qu'un champ désérialisé depuis le fil, et cette ligne
en est la source. La page consultée ne dit rien d'une valeur `-1` : la lecture
de `parse_price` (`crates/oxyn-llm/src/openai_compatible/wire.rs`) — qui rejette
tout prix négatif, donc `-1` compris — reste donc, pour ce point précis, non
confirmée par la documentation du fournisseur.

## Bancs d'essai — vérification du 2026-09-10

`criterion` est en **`0.8.2`**, relevée au registre par `cargo search criterion`
le 2026-09-10. Elle est adoptée à cette date, en `[dev-dependencies]` du
workspace, et n'entre dans le graphe que par les cibles `[[bench]]` de
`oxyn-query` et `oxyn-driver-sqlite`.

Fonctionnalités déclarées par la version publiée, relevées par
`cargo add --dry-run --dev criterion` : `cargo_bench_support`, `plotters` et
`rayon` sont actives par défaut ; `async`, `async_futures`, `async_smol`,
`async_tokio`, `csv_output`, `html_reports`, `real_blackbox` et `stable` ne le
sont pas. Aucune n'est activée en plus des défauts : les bancs qui ont besoin
d'un exécuteur construisent leur propre `tokio::runtime` et appellent
`block_on`, ce qui évite d'ajouter `async_tokio` — la crate n'a rien à savoir de
notre exécuteur. `real_blackbox` est inutile depuis que `std::hint::black_box`
est stable, et c'est celui que les bancs utilisent.

Deux points d'usage vérifiés à l'exécution, et non déduits de la documentation :

- un exécutable `criterion` lancé **sans** l'argument `--bench` se met en mode
  test et n'imprime aucune mesure. Il ne signale rien d'autre que `Success` :
  une campagne lancée ainsi paraît avoir tourné ;
- `BenchmarkGroup::sample_size` **écrase** l'argument `--sample-size` de la
  ligne de commande. Une valeur écrite dans le code n'est donc pas réglable à
  l'invocation.

La campagne de mesure elle-même, ses conditions et ses résultats sont dans
[PERFORMANCE](PERFORMANCE.md#campagne-de-mesure-du-2026-09-10) : ce fichier-ci ne
porte que les faits externes.

## Menus, fenêtres et DDL destructeur — vérification du 2026-09-25

Faits sur lesquels reposent
[ADR-0041](adr/0041-registre-d-actions-menus-et-raccourcis.md),
[ADR-0042](adr/0042-revue-sur-place-des-operations-destructrices.md) et
[ADR-0043](adr/0043-multi-fenetre.md). Côté Tauri, lus dans le source des
versions de `Cargo.lock` dépaquetées dans le registre local : `tauri` 2.11.5,
`tauri-utils` 2.9.3, `tauri-runtime-wry` 2.11.4, `wry` 0.55.1, `muda` 0.19.3 ;
côté front, `@codemirror/commands` 6.11.0, dépendance transitive de
`@uiw/react-codemirror`. Ce que tao fait de `terminate:` (Quit prédéfini, Dock,
fermeture de session) est au tableau d'[Interface Tauri et front](#interface-tauri-et-front),
et le comportement du dialogue de message sans fenêtre parente sous la liste
d'ADR-0037 : ni l'un ni l'autre n'est repris ici.

| Sujet | Fait vérifié | Source |
|---|---|---|
| Menu macOS par défaut | Sans menu fourni, `Builder::build` pose `Menu::default` (application, File, Edit, View, Window, Help) tant que `enable_macos_default_menu` vaut `true`, le défaut ; rien sous Windows et Linux | `tauri` `src/app.rs`, `src/menu/menu.rs` |
| Rôles système | Les entrées Edit de `muda` sont des sélecteurs AppKit (`copy:`, `paste:`, `cut:`, `selectAll:`, `undo:`, `redo:`) ; Quit est `terminate:`, Close Window `performClose:` | `muda` `src/platform_impl/macos/mod.rs` |
| Accélérateurs prédéfinis | Sous macOS, `Close Window` porte `⌘W`, `Quit` `⌘Q`, `Hide` `⌘H`, `Hide Others` `⌘⌥H` ; ailleurs, `Close Window` porte `Alt+F4` | `muda` 0.19.3 `src/items/predefined.rs` |
| Élément de menu | `set_text`, `set_enabled`, `set_accelerator` ; aucune infobulle ; un raccourci affiché est un raccourci lié | `muda` 0.19.3 |
| Permissions | `event` et `menu` sont des permissions de cœur : `listen` côté JS en exige une ; les commandes de l'application et les `Channel` n'y sont pas soumis | `tauri` 2.11.5 ; description de `capabilities/main.json` |
| Barre de titre | `titleBarStyle` ne vaut que pour macOS | `tauri-utils` `src/config.rs` |
| Zoom et accélérateurs | `zoomHotkeysEnabled` vaut `false` ; `back_forward_navigation_gestures` vaut `false` ; `with_browser_accelerator_keys` de `wry` (vrai par défaut sous WebView2) n'est pas exposé par `tauri-runtime-wry` ; `allowLinkPreview` vaut `true` sous macOS | `tauri-utils` `src/config.rs`, `wry`, `tauri-runtime-wry` |
| Dépôt de fichiers | `dragDropEnabled` vaut `true` ; « Disabling it is required to use HTML5 drag and drop on the frontend on Windows » | `tauri-utils` `src/config.rs` |
| Capability | Le champ `windows` accepte un motif glob | `tauri-utils` `src/acl/capability.rs` |
| Identité de l'appelant | Une commande peut recevoir la `Webview` ou la `WebviewWindow` qui l'invoque, fournie par l'environnement d'exécution | `tauri` `src/webview/mod.rs`, `src/webview/webview_window.rs` |
| `Channel` | Livre à la webview qui l'a créé, et à elle seule | `tauri` `src/ipc/channel.rs` |
| Événements de menu | L'écouteur est global à l'application, quelle que soit la fenêtre | `tauri`, doc de `on_menu_event` |
| Création de fenêtre | « deadlocks when used in a synchronous command or event handlers » sous Windows | `tauri`, doc de `WebviewWindowBuilder::new` |
| Dernière fenêtre | Sa destruction émet `RunEvent::ExitRequested { code: None }`, empêchable ; `RunEvent::Reopen` n'existe que sous macOS | `tauri-runtime-wry` `src/lib.rs` ; `tauri` `src/app.rs` |
| Dialogue avec fenêtre parente | Sous macOS, `rfd` présente un dialogue de message sans parent par `utils::sync_pop_dialog` ou `async_pop_dialog` — l'alerte `CFUserNotificationDisplayAlert` d'ADR-0037 —, et avec parent par un `NSAlert` en feuille (`beginSheetModalForWindow`) | `rfd` 0.16.0 `src/backend/macos/message_dialog.rs`, `show` et `show_async` |
| Gabarit de fenêtre | Une fenêtre déclarée `"create": false` sert de gabarit à `WebviewWindowBuilder::from_config` | `tauri-utils`, `WindowConfig::create` |
| PostgreSQL | `DROP TABLE` vaut `RESTRICT` ; `TRUNCATE` refuse une table référencée et est transactionnel ; le DDL est transactionnel sauf base et tablespace | [sql-droptable](https://www.postgresql.org/docs/18/sql-droptable.html), [sql-truncate](https://www.postgresql.org/docs/18/sql-truncate.html), [wiki](https://wiki.postgresql.org/wiki/Transactional_DDL_in_PostgreSQL:_A_Competitive_Analysis) — version documentaire 18 |
| Redshift | `TRUNCATE` « commits the transaction in which it is run » ; `DROP TABLE` vaut `RESTRICT` | [r_TRUNCATE](https://docs.aws.amazon.com/redshift/latest/dg/r_TRUNCATE.html), [r_DROP_TABLE](https://docs.aws.amazon.com/redshift/latest/dg/r_DROP_TABLE.html) |
| MySQL 8.4 | `DROP TABLE`, `TRUNCATE TABLE`, `RENAME TABLE`, `ALTER TABLE` valident implicitement | [implicit-commit](https://dev.mysql.com/doc/refman/8.4/en/implicit-commit.html) |
| SQLite | Pas de `TRUNCATE` ; `DROP TABLE` passe malgré une vue dépendante, et malgré des lignes filles quand `foreign_keys` vaut `0` ; `DROP` se défait par `ROLLBACK` | constaté avec le client `sqlite3` 3.51.0 ; le driver embarque 3.50.2, d'où l'exigence d'ADR-0042 : un test d'intégration du driver avant de déclarer chaque drapeau |

## Licences — vérification du 2026-09-25

Faits sur lesquels repose [ADR-0044](adr/0044-licence-gpl-et-contrat-apache.md).

### Le dépôt

| Fait | Valeur | Source |
|---|---|---|
| Dépôt | `so-keyldzn/oxyn`, privé, licence détectée par GitHub : Apache-2.0 avant ce changement | `gh repo view so-keyldzn/oxyn` |
| Ancienne valeur de `repository` | `https://github.com/keyldzn/oxyn`, qui ne se résout pas | `gh repo view keyldzn/oxyn` : « Could not resolve to a Repository » |

### Le texte de la GPL

| Fait | Valeur | Source |
|---|---|---|
| `LICENSE-GPL` | GNU General Public License, version 3, 29 June 2007 ; 674 lignes ; SHA-256 `3972dc9744f6499f0f9b2dbf76696f2ae7ad8af9b23dde66d6af86c9dfb36986` | téléchargé tel quel depuis <https://www.gnu.org/licenses/gpl-3.0.txt> le 2026-09-25 |

### Compatibilité avec la GPLv3 des licences acceptées

Relevé dans la liste de la FSF, <https://www.gnu.org/licenses/license-list.html>,
le 2026-09-25. L'ancre est celle de l'entrée dans la page.

| Licence SPDX | Entrée FSF | Verdict |
|---|---|---|
| `Apache-2.0` | `#apache2` | compatible avec la GPLv3 (pas avec la GPLv2) |
| `Apache-2.0 WITH LLVM-exception` | — | l'exception ajoute des permissions à Apache-2.0 et n'en retire aucune |
| `MIT` | `#Expat`, `#X11License` | compatible |
| `MIT-0` | `#Expat0` | compatible, comme `#Zero-BSD` |
| `BSD-2-Clause` | `#FreeBSD` | compatible |
| `BSD-3-Clause` | `#ModifiedBSD` | compatible |
| `ISC` | `#ISC` | compatible |
| `Zlib` | `#ZLib` | compatible |
| `Unicode-3.0` | `#Unicodev3` | compatible avec toutes les versions de la GPL |
| `CC0-1.0` | `#CC0` | compatible |
| `MPL-2.0` | `#MPL-2.0` | compatible par la section 3.3, sauf fichier marqué « Incompatible With Secondary Licenses » |
| `BSL-1.0` | `#boost` | compatible |
| `NCSA` | `#NCSA` | compatible |
| `CDLA-Permissive-2.0` | absente de la liste | voir ci-dessous |
| `0BSD` (npm) | `#Zero-BSD` | compatible |
| `Unlicense` (npm) | `#Unlicense` | compatible |
| `Python-2.0` (npm) | `#Python` | compatible (versions 2.0.1, 2.1.1 et suivantes) |
| `CC-BY-4.0` (npm) | `#ccby` | compatible avec toutes les versions de la GPL, et à ne pas employer pour du logiciel |
| `OFL-1.1` (npm) | `#SILOFL` | licence libre pour polices. Sa seule exigence inhabituelle, vendre la police avec un logiciel et non seule, est « inoffensive » selon la FSF |

**`CDLA-Permissive-2.0`**, lu dans le texte SPDX
(<https://github.com/spdx/license-list-data>, `text/CDLA-Permissive-2.0.txt`,
le 2026-09-25). La licence porte sur des données. Sa seule condition de partage
est la § 2.1 : « makes available the text of this agreement with the shared
Data ». La § 3.1 n'impose rien aux résultats. C'est une mention à reproduire,
ce que la GPLv3 permet d'exiger (§ 7 b). **Cette conclusion est la nôtre, pas
celle de la FSF.** Les mentions tierces de l'application reproduisent ce texte.

**`MPL-2.0` dans le graphe Rust.** Les crates concernées sont `cssparser`
0.36.0, `cssparser-macros` 0.6.1, `dtoa-short` 0.3.5, `option-ext` 0.2.0 et
`selectors` 0.36.1, toutes tirées par Tauri. Aucun de leurs fichiers `.rs` ne
porte la mention « Incompatible With Secondary Licenses ». La recherche a été
faite dans `~/.cargo/registry/src` le 2026-09-25 ; les seules occurrences sont
dans le texte de la licence elle-même, qui cite la mention en modèle.

### Le modèle du CLA

| Fait | Valeur | Source |
|---|---|---|
| Modèle | Apache Software Foundation, *Individual Contributor License Agreement* V2.2 | <https://www.apache.org/licenses/icla.pdf>, téléchargé le 2026-09-25 |
| Licence accordée (§ 2) | « perpetual, worldwide, non-exclusive, no-charge, royalty-free, irrevocable copyright license to reproduce, prepare derivative works of, publicly display, publicly perform, sublicense, and distribute » | même document |
| Licence de brevet (§ 3) | limitée aux revendications nécessairement enfreintes par la contribution ; résiliée pour qui intente une action en contrefaçon | même document |

Les écarts entre `CLA.md` et ce modèle sont listés à la fin de `CLA.md`. Le
droit de sous-licencier (« sublicense ») est ce qui permet de relicencier ; la
§ 2 de `CLA.md` l'écrit en clair. La cession (§ 9) n'existe pas dans le modèle.
**Ce texte n'a pas été relu par un avocat** : la relecture est à faire avec la
cession des droits à la société.

### Les mentions tierces

| Fait | Valeur | Source | Vérifié le |
|---|---|---|---|
| `cargo-about` | **0.9.2**, publiée le 2026-08-18 ; `MIT OR Apache-2.0` ; `rust-version` 1.88.0 | [crates.io](https://crates.io/crates/cargo-about) | 2026-09-25 |
| Binaires de `cargo-about` 0.9.2 | `aarch64-apple-darwin` : SHA-256 `ae72f0df0c399a1e96336f696fa55b1b28679fd725632eba8cf8e4568467cc3e` ; `x86_64-unknown-linux-musl` : `9099a59e820c38a68b9d65f300662a567d56562f9a10f6aa4c7e86c17c2566af`. Aucun binaire `x86_64-apple-darwin` | [release GitHub](https://github.com/EmbarkStudios/cargo-about/releases/tag/0.9.2), fichiers `.sha256` recalculés au téléchargement | 2026-09-25 |
| `private = { ignore = true }` | existe dans la configuration de `cargo-about` comme dans celle de `cargo-deny` ; sans lui, chaque crate du workspace sous GPL fait échouer la génération (« failed to satisfy license requirements ») | exécution de `cargo about generate` | 2026-09-25 |
| `pnpm licenses list --prod --json` | un objet `{ licence: [{ name, versions, paths, license, … }] }` ; `Unknown` quand `package.json` n'a pas de champ `license` ; fonctionne avec pnpm 11.1.2 | exécution dans `apps/desktop` | 2026-09-25 |

Trois constats de la première génération, le 2026-09-25 :

- 417 crates et 413 paquets npm, 297 textes distincts, 1,18 Mo de JSON. Vite
  en fait un morceau à part, de 100 ko compressé, chargé à l'ouverture de la
  section About ;
- `cargo about generate` prend environ 20 s et 100 s de CPU, surtout pour
  identifier les textes de licence. C'est pourquoi `make front-build` ne
  régénère le fichier que si `Cargo.lock`, `pnpm-lock.yaml`, `deny.toml` ou la
  configuration npm ont changé ;
- onze paquets npm ne livrent aucun fichier de licence, dont
  `@uiw/react-codemirror` et `embla-carousel`. Les mentions reprennent alors
  l'expression SPDX déclarée.
