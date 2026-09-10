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

## GPUI

| Fait | Valeur | Source | Vérifié le |
|---|---|---|---|
| Dernière version publiée | `0.2.2`, publiée le 2025-10-22 | crates.io API | 2026-09-05 |
| Licence | Apache-2.0 | crates.io API | 2026-09-05 |
| Édition | 2024 | crates.io API | 2026-09-05 |
| MSRV déclaré | **aucun** (`rust_version` absent) | crates.io API | 2026-09-05 |
| Dépendances normales non optionnelles | 65 | crates.io API `/dependencies` | 2026-09-05 |
| Dépôt de développement | `github.com/zed-industries/zed` | crates.io API | 2026-09-05 |
| Features | `default`, `inspector`, `leak-detection`, `macos-blade`, `runtime_shaders`, `screen-capture`, `test-support`, `wayland`, `windows-manifest`, `x11` | crates.io API | 2026-09-05 |

> **Deux pièges vérifiés.**
> 1. La dernière publication remonte à **près de onze mois** alors que le
>    développement continue dans le dépôt Zed. La version épinglée ne recevra
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
| Marque Oxyn | Même Figma, composant `149:22199`, relu après la mise à jour des couleurs | Deux SVG 32 × 32 px selon le thème, fragment orangé et marges internes conservés |
| Geist | [vercel/geist-font](https://github.com/vercel/geist-font/tree/10dc7658f13c38a474cde201bb09a4617267545b/fonts/Geist/ttf) | Regular, Medium et SemiBold, TTF embarqués sous SIL OFL |

Nœuds, dimensions et SHA-256 : [icônes](../assets/ui/provenance.json) et
[polices](../assets/fonts/provenance.json). Les notices de licence restent avec
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
| `criterion` | `0.8.2` | 2026-02-04 | 2026-09-05 |
| `gpui` | `0.2.2` | 2025-10-22 | 2026-09-05 |

> `gpui` figure ici pour être couverte par le vérificateur automatique ; elle est
> adoptée, non candidate — voir la section GPUI ci-dessus et
> [ADR-0009](adr/0009-source-dependance-gpui.md).

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
plus détaillée, dans [`assets/ui/provenance.json`](../assets/ui/provenance.json).

Les dimensions, espacements et rayons sont publiés dans
`crates/oxyn-ui/src/theme.rs` — `Metrics`, `Spacing`, `Radii` — où chaque champ
cite son nœud. **La typographie ne l'est que partiellement** : `Typography` ne
porte ni graisses ni styles nommés, et deux de ses valeurs restent sans source —
voir la table des manques ci-dessous.

| Fait | Valeur | Source | Vérifié le |
|---|---|---|---|
| Échelle d'espacement | `space/0`=0, `space/4`=4, `space/8`=8, `space/12`=12, `space/16`=16, `space/24`=24 | Nœuds `13:291` et `47:8222` | 2026-09-07 |
| Rayons de coin | `radius/6`=6, `radius/8`=8, `radius/full`=999 | idem | 2026-09-07 |
| Hauteur de barre d'outils | `52` | Nœud `47:8422` « Workspace toolbar » | 2026-09-07 |
| Largeur du panneau latéral déplié | `280` | Nœuds `8:4`, `13:292` | 2026-09-07 |
| Largeur du panneau latéral replié | `64` | Nœuds `13:165`, `13:484` | 2026-09-07 |
| Hauteur d'un contrôle | `38` | Nœuds `47:8461`, `47:8465`, `47:8469` | 2026-09-07 |
| Familles et graisses | Geist — Title 24/32 SemiBold, Body 13/20 Regular, Label 13/20 Medium, Caption 11/16 Regular, Section 11/16 Medium | Nœuds `13:291`, `47:8222` | 2026-09-07 |

### Les binaires embarqués et leurs licences

Ces fichiers sont liés au binaire par `include_bytes!` dans
`crates/oxyn-ui/src/icons.rs`. Ils portent un commit amont exact, comme une
dépendance de code.

| Fait | Valeur | Source | Vérifié le |
|---|---|---|---|
| Police d'interface | Geist Regular, Medium, SemiBold, commit `10dc7658f13c38a474cde201bb09a4617267545b` | [vercel/geist-font](https://github.com/vercel/geist-font) ; SHA-256 dans [`assets/fonts/provenance.json`](../assets/fonts/provenance.json) | 2026-09-07 |
| Licence de la police | SIL Open Font License 1.1 | `assets/fonts/OFL.txt`, `assets/fonts/LICENSE.txt` | 2026-09-07 |
| Icônes | Hugeicons Stroke Rounded, 12 glyphes, commit `f9dbcca8d72cc2777a0ccd873c274d9bf7a153e6` | [hugeicons/hugeicons-static](https://github.com/hugeicons/hugeicons-static) ; SHA-256 dans [`assets/ui/provenance.json`](../assets/ui/provenance.json) | 2026-09-07 |
| Licence des icônes | **Aucune licence MIT attribuée** ; le README amont autorise l'usage tel quel, sans mention d'un droit de redistribution | [`assets/ui/HUGEICONS-UPSTREAM-README.txt`](../assets/ui/HUGEICONS-UPSTREAM-README.txt) | 2026-09-07 |

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
