# Oxyn

A desktop workspace, with a native high-performance backend, to explore, query
and manage any kind of database — relational, analytical, NoSQL, vector, graph.
AI agents help users understand schemas and queries **alongside** them, never in
their place. Audience: data professionals, people who read PostgreSQL error
messages.

**Language.** The repository is written in **English**: code, identifiers,
comments, error messages, `///`, documentation, ADRs, commit messages, pull
requests. Two exceptions, set by
[ADR-0047](docs/adr/0047-english-as-the-repository-language.md): the documents
of `docs/` and ADRs 0001 to 0046 written in French stay authoritative as they
are, until they are translated; and [`i18n/fr/`](i18n/README.md) holds French
mirrors of the English documents. **English is authoritative**; a mirror is a
translation, never a place where a rule is decided.

**The state of the repository is not here**: it is injected at every session by
`.claude/hooks/contexte_session.py`. This file holds nothing perishable.

## Documentation is authoritative

| Document | Authoritative on |
|---|---|
| [docs/VISION.md](docs/VISION.md) | the product scope and its principles |
| [docs/ARCHITECTURE.md](docs/ARCHITECTURE.md) | crates, dependencies, command bus, threads |
| [docs/DRIVER-CONTRACT.md](docs/DRIVER-CONTRACT.md) | what every driver guarantees |
| [docs/AI-PROVIDERS.md](docs/AI-PROVIDERS.md) | what crosses the AI boundary |
| [docs/PLUGIN-CONTRACT.md](docs/PLUGIN-CONTRACT.md) | what a plugin can do |
| [docs/SECURITY.md](docs/SECURITY.md) | secrets, connections, input surface, `unsafe` |
| [docs/PERFORMANCE.md](docs/PERFORMANCE.md) | the numeric budgets |
| [docs/UX-SPEC.md](docs/UX-SPEC.md) | interface behaviors |
| [docs/RESEARCH-NOTES.md](docs/RESEARCH-NOTES.md) | every external version, sourced and dated |
| [docs/IMPLEMENTATION-PLAN.md](docs/IMPLEMENTATION-PLAN.md) | the phases and their exit gates |
| [docs/adr/](docs/adr/) | decisions that are expensive to undo |

**When the code and one of these documents contradict each other, it is a bug:
report it, do not settle it alone.**

## Stack

Checked on 2026-09-05, interface on 2026-09-15 — [details and sources](docs/RESEARCH-NOTES.md).

| | Version | Worth knowing |
|---|---|---|
| Rust | pinned by `rust-toolchain.toml` ([ADR-0008](docs/adr/0008-chaine-outils-rust.md)) | the dev machine runs `1.89.0`, stable is `1.98.1` |
| Edition | **2024** | requires Rust ≥ 1.85 |
| Interface | **Tauri 2** + TanStack Start in **SPA mode** + shadcn/ui on **Base UI** ([ADR-0029](docs/adr/0029-interface-tauri-shadcn.md)) | `apps/desktop` (pnpm) served by `crates/oxyn-desktop`; no server, the backend is Rust |
| Component tests | Storybook 10 + `addon-vitest` + `addon-a11y` | a story is a test, axe included; Vitest stays on **4** |
| Results | Apache Arrow ([ADR-0002](docs/adr/0002-arrow-result-model.md)) | `RecordBatch` end to end |

No version is written from memory: see [I-12](#i-12).

## Commands

| | |
|---|---|
| `make qualite` | **the** quality gate: foundation, dated TODOs, front end (format, lint, types, stories, build), format, clippy, tests, doc. Nothing is done without it, and CI calls nothing else, in parallel jobs |
| `make verif-rapide` | day-to-day work: checks only what changed since `origin/main` — modified crates, modified front-end files, foundation — and says what it leaves to CI. It is **not** conclusive: `make qualite` is |
| `make desktop-dev` | the Tauri application with hot reload, on a temporary workspace |
| `script/nouvelle-crate` | creates an already compliant crate — see the rules trap below |
| [`/plan`](.claude/commands/plan.md) [`/implementer`](.claude/commands/implementer.md) [`/relire`](.claude/commands/relire.md) | the everyday cycle |
| [`/driver`](.claude/commands/driver.md) [`/commande`](.claude/commands/commande.md) [`/ecran`](.claude/commands/ecran.md) [`/conformite-shadcn`](.claude/commands/conformite-shadcn.md) | the moves that have a contract to honor |
| [`/adr`](.claude/commands/adr.md) [`/versions`](.claude/commands/versions.md) [`/benchmark`](.claude/commands/benchmark.md) [`/securite`](.claude/commands/securite.md) | the rare moves that are easy to get wrong |

Full list: [.claude/README.md](.claude/README.md).

## Invariants

Thirteen prohibitions. Violating them is **silent**: nothing fails at the moment
of the mistake. Each one points to the document that grounds it.

<a id="i-01"></a>**I-01 — Nothing bypasses the command bus.** The UI, an agent and
a plugin emit `Command`s; none of them calls a driver. A second execution path,
once created, is never audited like the first — and that is the one the AI will
take. → [ADR-0004](docs/adr/0004-command-bus.md)

<a id="i-02"></a>**I-02 — No write on a `production` connection without a
confirmation that names the connection.** For an `Actor::Agent`, it is a
refusal, not a stronger confirmation: a confirmation ends up being clicked. A
connection with no environment set counts as `production`, never the other way
round. → [SECURITY](docs/SECURITY.md#marquage-des-connexions)

<a id="i-03"></a>**I-03 — No secret in a log, a displayed error, a crash report, a
workspace file, an AI prompt or the clipboard.** All six channels count;
forgetting one is enough. Checkable corollary: **no `#[derive(Debug)]` on a type
that carries a secret** — it is the `tracing::debug!("{cfg:?}")` added six
months later that leaks. → [SECURITY](docs/SECURITY.md#secrets)

<a id="i-04"></a>**I-04 — Nothing reaches an AI prompt outside the single
gateway that applies the connection's tier.** The tier is attached to the
connection, not to the session or the provider: otherwise a setting chosen on a
test database applies to the customer database opened three days later.
→ [AI-PROVIDERS](docs/AI-PROVIDERS.md) · [ADR-0006](docs/adr/0006-ai-privacy-tiers.md)

<a id="i-05"></a>**I-05 — No I/O, no network, no `block_on` on the UI thread.** A
30 s query there freezes the whole window; the user concludes it crashed and
kills the process, losing unsaved work.
→ [ARCHITECTURE](docs/ARCHITECTURE.md#le-modèle-de-threads)

<a id="i-06"></a>**I-06 — No result is materialized in full.** Streamed
`RecordBatch`, bounded `ResultBuffer`, spill to disk. A `SELECT *` over 50
million rows triggers the OOM killer: on macOS the process dies without a trace,
after a simple click on a table.
→ [ADR-0002](docs/adr/0002-arrow-result-model.md) · [PERFORMANCE](docs/PERFORMANCE.md#budgets-de-mémoire)

<a id="i-07"></a>**I-07 — No model output is executed directly.** It becomes a
`Command` carrying `Actor::Agent` and goes through the `PolicyGate`. Including
what "only reads": `EXPLAIN ANALYZE` actually runs the query it analyzes,
`DELETE` included.
→ [AI-PROVIDERS](docs/AI-PROVIDERS.md#ce-quon-fait-des-réponses)

<a id="i-08"></a>**I-08 — No crate other than `oxyn-desktop` depends on `tauri`.**
A toolkit type imported into `oxyn-core` "just for one field" permanently removes
the possibility of a CLI, of headless tests, and of an interface change — the
one [ADR-0029](docs/adr/0029-interface-tauri-shadcn.md) made by removing GPUI
without touching the core.
→ [ARCHITECTURE](docs/ARCHITECTURE.md#le-sens-des-dépendances)

<a id="i-09"></a>**I-09 — No `unwrap`, `expect`, `panic!`, `unreachable!`, slice
indexing or overflowing `as` on a path reachable from a server response.** A
server returns whatever it wants: an unknown type, a `NULL` where the schema
forbids it, an invalid encoding. The panic kills the application.
→ [DRIVER-CONTRACT](docs/DRIVER-CONTRACT.md#1-il-ne-panique-jamais-sur-une-entrée-venue-du-serveur)

<a id="i-10"></a>**I-10 — SQL composed by Oxyn never concatenates a received
identifier.** Quoting by the driver, bound values. The SQL *the user writes* is
sent as is — that is the feature. A table named `"users"; DROP TABLE audit; --`
is legal in PostgreSQL: a preview built by concatenation runs the drop on click.
→ [DRIVER-CONTRACT](docs/DRIVER-CONTRACT.md#6-il-échappe-tout-identifiant-quil-compose)

<a id="i-11"></a>**I-11 — No closed persistence format.** What Oxyn writes —
workspace, session, export — is readable without Oxyn. "Open by default" is not a
stance: a user who cannot recover their work without the product is captive.
→ [VISION](docs/VISION.md)

<a id="i-12"></a>**I-12 — No version, no external limit copied from memory.**
Checked against the registry, dated in [RESEARCH-NOTES](docs/RESEARCH-NOTES.md),
re-checked by [`/versions`](.claude/commands/versions.md). A plausible and wrong
value shows up neither at compile time, nor in tests, nor in review.

<a id="i-13"></a>**I-13 — An ambiguous error is never retried.** A client-side
timeout during a write is not a transient error: the server may have applied it.
Replaying creates a duplicate in the user's data, with no error message
anywhere.
→ [DRIVER-CONTRACT](docs/DRIVER-CONTRACT.md#4-il-distingue-trois-familles-derreurs-et-il-les-classe)

## Code organization

The tree and the direction of dependencies are authoritative in
[ARCHITECTURE](docs/ARCHITECTURE.md#le-découpage). What holds everywhere:

- **a crate carries one subject.** `utils`, `common`, `helpers`, `misc` are
  forbidden: a catch-all name is a failed split that becomes the universal
  coupling point;
- **one driver per protocol, not per product** — Redshift ≡ PostgreSQL
  ([ADR-0003](docs/adr/0003-driver-capabilities.md));
- **no dead code, no code commented out "just in case"**: git remembers it;
- **no `TODO` without a date** and without naming what unblocks it;
- **no abstraction for a single caller** — a trait with a single implementation
  that is not a boundary is an indirection, not a decoupling;
- **a comment says *why*.** What the code does, the code says;
- **vigilance threshold**: beyond about 400 lines, a file probably carries two
  subjects. It is a signal to look at, not a mechanical rule.

## On-demand rules

`.claude/rules/*.md` carries the conventions of a directory. They load when
Claude **reads** a file matching their `paths:`.

| Rule | `paths:` |
|---|---|
| [rust.md](.claude/rules/rust.md) | `**/*.rs` |
| [drivers.md](.claude/rules/drivers.md) | `drivers/oxyn-driver-*/**`, `crates/oxyn-driver/**` |
| [front.md](.claude/rules/front.md) | `apps/desktop/**`, `crates/oxyn-desktop/**` |
| [ia.md](.claude/rules/ia.md) | `crates/oxyn-ai/**`, `crates/oxyn-llm/**` |
| [tests.md](.claude/rules/tests.md) | `**/*_tests.rs`, `**/tests.rs`, `**/tests/**`, `**/benches/**` |
| [documentation.md](.claude/rules/documentation.md) | `docs/**/*.md`, `*.md`, `.claude/**/*.md`, `i18n/**/*.md` |
| [manifestes.md](.claude/rules/manifestes.md) | `**/Cargo.toml`, `Cargo.lock`, `rust-toolchain.toml`, `Makefile`, `deny.toml`, `clippy.toml`, `.cargo/config.toml`, `.config/nextest.toml`, `renovate.json5`, `script/*`, workflows |

> **The trap to know.** A `paths:` rule loads when Claude *reads* a matching
> file, **not when it creates one**. The first file of a new directory is
> therefore written without its rule, and a manifest without
> `[lints] workspace = true` makes nothing fail. Two things compensate, and
> neither is optional: `script/nouvelle-crate`, which writes an already
> compliant skeleton, and the commands of `.claude/commands/`, which load the
> procedure explicitly — `/driver`, `/commande`, `/ecran`.

## What is executed

`.claude/hooks/` **refuses** what this file can only ask for: forbidden writes,
shell workarounds, commit format, injection of the real state at startup. A hook
is not a reminder, it is a wall. Details and protocol:
[.claude/hooks/README.md](.claude/hooks/README.md).

Hooks only apply to a Claude session. What applies to **everyone**, humans
included, goes through `make qualite` — called by CI
([.github/workflows/qualite.yml](.github/workflows/qualite.yml)) in parallel
jobs, which adds no check of its own and which `make socle` verifies misses
none. On a pull request, it skips the jobs whose area — Rust, front end — is not
touched, and the aggregate `qualite` job is the only check to read; on `main`,
everything always runs
([ADR-0045](docs/adr/0045-ci-selective-sur-les-pull-requests.md)):

| What refuses | The invariant held |
|---|---|
| [clippy.toml](clippy.toml) | forbidden call paths ([I-03](#i-03), [I-05](#i-05), [I-09](#i-09)) |
| `.claude/verifier_socle.py` | `tauri*` outside `oxyn-desktop`, `gpui` anywhere, a manifest outside the workspace ([I-08](#i-08)); a French mirror older than its English original |
| `make front` | a component without a passing story, an accessibility violation, a wrong type or lint in `apps/desktop` |
| `script/verifier-todo` | a remaining-work marker without a deadline |
| `script/licences-tierces` | an npm dependency shipped under a license that `deny.toml` does not accept |
| [renovate.json5](renovate.json5) | a version copied from memory ([I-12](#i-12)) |
