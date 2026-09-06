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

## Écosystème MCP

| Fait | Valeur | Source | Vérifié le |
|---|---|---|---|
| Serveurs de référence maintenus | `everything`, `fetch`, `filesystem`, `git`, `memory`, `sequential-thinking`, `time` | `github.com/modelcontextprotocol/servers` | 2026-09-05 |
| Serveurs de référence **archivés** | `postgres`, `sqlite`, `github` | idem | 2026-09-05 |

> Conséquence directe : il n'existe **aucun** serveur MCP officiel pour
> PostgreSQL ni SQLite. Tout serveur de base de données branché sur Oxyn serait
> un serveur tiers, à auditer. Le raisonnement complet est dans
> [MCP.md](MCP.md).
