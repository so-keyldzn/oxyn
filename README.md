# Oxyn

A native, high-performance workspace to explore, query and manage any kind of
database — relational, analytical, NoSQL, vector, graph. AI agents help users
understand schemas and queries **alongside** them, never in their place.

Intended audience: data professionals, people who read PostgreSQL error
messages.

The scope and the principles are authoritative in [docs/VISION.md](docs/VISION.md).
The real progress is in
[docs/IMPLEMENTATION-PLAN.md](docs/IMPLEMENTATION-PLAN.md) — this README does
not duplicate it, because a copy of the progress is stale the next day.

## What the repository contains

15 crates in a single Cargo workspace: `crates/` for the core, the interface and
the binary; `drivers/` for the protocol implementations. The split and the
direction of dependencies are authoritative in
[docs/ARCHITECTURE.md](docs/ARCHITECTURE.md).

## Building

Prerequisites: macOS 13 or later, `python3`, and nothing else — the Rust
toolchain is pinned by `rust-toolchain.toml` and rustup installs it by itself on
the first `cargo` ([ADR-0008](docs/adr/0008-chaine-outils-rust.md)).

```sh
make app      # assembles target/debug/Oxyn.app
make lancer   # assembles then opens the application
```

`PROFIL=release` produces the optimized bundle. The `.app` bundle is not a
release step: without it, macOS treats the binary as an accessory — no Dock, no
activation, no correct keyboard focus.

## The quality gate

```sh
make qualite
```

It is the **only** entry point: CI calls its targets in parallel jobs
([.github/workflows/qualite.yml](.github/workflows/qualite.yml)), the `Stop` hook
reminds of it, the definition of "done" rests on it. A check added elsewhere is
an optional check.

It chains: consistency of the steering foundation, dated `TODO`s, format,
clippy, tests, documentation. `make aide` lists the other targets.

## Where the truth lives

| | |
|---|---|
| [docs/](docs/README.md) | the domain — vision, architecture, contracts, security, budgets, ADRs |
| [CLAUDE.md](CLAUDE.md) | the map of the repository and the thirteen invariants |
| [.claude/](.claude/README.md) | the way of working — rules, commands, agents, hooks |
| [AGENTS.md](AGENTS.md) | the equivalent entry point for Codex |
| [i18n/fr/](i18n/README.md) | French mirrors of the English documents |

**When the code and a document of `docs/` contradict each other, it is a
bug**: report it, do not settle it alone.

## Language

The repository is written in **English**: code, identifiers, comments, error
messages, `///`, documentation, ADRs, commit messages and pull requests
([ADR-0047](docs/adr/0047-english-as-the-repository-language.md)).
[`i18n/fr/`](i18n/README.md) holds French mirrors; **English is
authoritative**, and `make qualite` refuses a mirror older than its original.

## License

Copyright 2026 Nicolas Boromée.

The application is under **GPL-3.0-or-later** ([LICENSE-GPL](LICENSE-GPL)). The
four crates a third-party driver must link (`oxyn-core`, `oxyn-catalog`,
`oxyn-data` and `oxyn-driver`) are under **Apache-2.0**
([LICENSE-APACHE](LICENSE-APACHE)). A driver or plugin author therefore chooses
their own license. [NOTICE](NOTICE) says which license covers which part, and
[ADR-0044](docs/adr/0044-licence-gpl-et-contrat-apache.md) says why.

All the code in this repository is usable without an account or a subscription.
What will be paid for are optional services attached to an account, such as
hosted AI or synchronization.

An external contribution requires accepting the [CLA](CLA.md): see
[CONTRIBUTING](CONTRIBUTING.md).
