# ADR-0067 — On a pull request, CI checks only the Rust packages it changes and those that depend on them

**Status:** accepted · **Date:** 2026-10-24

**Refines:** [ADR-0045](0045-ci-selective-sur-les-pull-requests.md), on one
point: inside the Rust area, a pull request no longer checks the whole
workspace. It reverses the alternative ADR-0045 rejected, "compute the
dependent crates and test only them in CI". Everything else in ADR-0045 stays
in force: the areas, the full run on `main`, the aggregate `qualite` job.

## Context

Under ADR-0045, a pull request is selective **by area** only. Any file under
`crates/` or `drivers/` runs the whole of `make rust` — `cargo fmt`, clippy
`--workspace`, nextest `--workspace --all-features`, the documentation tests,
`cargo doc`, `cargo deny`, `package-contracts` — on macOS and Linux, plus the
`serveurs` job, which replays every database-server suite (PostgreSQL, TLS
PostgreSQL, MySQL, ClickHouse; MongoDB, Redis and Elasticsearch are coming).

On 2026-10-23 and 2026-10-24, with six driver pull requests open at once, the
macOS `rust` job took more than 60 minutes per run, and the runner queue
became the bottleneck of the flow. A pull request that only touches the
ClickHouse driver recompiled and retested the SQLite driver, the AI workspace
and the plugin host, none of which can observe the change.

The maintainer stated the rule twice: **"we test only the parts we code, not
all the code"**.

What makes the computation safe now, when ADR-0045 judged it hard: the graph
between workspace members is entirely local. `cargo metadata --no-deps
--format-version 1` lists, for every member, each dependency it **declares** —
normal, dev and build, for every platform, optional ones included — with the
`path` of those that are members. It reads only the manifests: no registry, no
network, no download, no build; it answers in about 0.1 s on this workspace
(measured on 2026-10-24 with the pinned toolchain).

## Decision

**On the `pull_request` event only**, `script/zones-ci` computes, after the
areas, the set of **Rust packages** to check, and writes it to the
`rust_packages` output of the `zones` job (names separated by spaces):

1. every modified file is attributed: a file under `crates/` or `drivers/` to
   the workspace package whose directory holds it (the deepest one); a file
   under `apps/desktop/` to `oxyn-desktop`, which compiles the front end in
   (`tauri::generate_context!`, `include_str!` of
   `apps/desktop/src/lib/actions/actions.json`); a documentation file
   (ADR-0045's `docs` area) to nothing;
2. the set is closed over the **reverse dependencies** between members, read
   from `cargo metadata --no-deps`: every declared edge counts, dev-dependencies
   included — `oxyn-exec`'s tests use the PostgreSQL driver, so a PostgreSQL
   change checks `oxyn-exec`;
3. the diff is read with `git diff --name-only --no-renames`: a file moved
   from one package to another counts against both.

**Fail-safe: an empty value means every package**, exactly the behaviour of
ADR-0045. The value is empty when:

* a file is cross-cutting or unclassified by ADR-0045 — the root `Cargo.toml`
  and `Cargo.lock`, `.cargo/`, `.config/`, `rust-toolchain.toml`,
  `clippy.toml`, `deny.toml`, the `Makefile`, `script/`, `.github/`, any
  unknown file;
* a file under `crates/` or `drivers/` belongs to no package of the merge
  commit — a deleted crate, a loose file;
* `cargo metadata` fails or returns something unreadable;
* the closure reaches every package anyway;
* the event is not `pull_request`: a push to `main` and `workflow_dispatch`
  always check everything.

The `Makefile` reads one optional variable, **`OXYN_PACKAGES`**. Empty or
unset, every command is byte for byte the previous one: `make qualite` and
`make rust` do not change locally. Set, it narrows:

| Target | With `OXYN_PACKAGES="a b"` |
|---|---|
| `lint` | `cargo clippy -p a -p b --all-targets --all-features` |
| `test` | `cargo nextest run -p a -p b --all-features`, then `cargo test --doc -p a -p b`; the documentation tests are skipped, and the target says so, when no selected package has a `src/lib.rs` — `cargo test --doc` refuses a selection without a library, which is `oxyn-desktop` alone |
| `doc` | `cargo doc -p a -p b --no-deps --all-features` |
| `test-serveurs` | `cargo nextest run -p <the selected packages among SERVER_PACKAGES>` with the same filter; nothing when none is selected |
| `format`, `deny`, `package-contracts`, `socle`, `todo`, `versions` | unchanged: they are cheap, or do not reason package by package |

The workflow passes the value as an environment variable to the existing
`make rust` and `make test-serveurs` steps; it calls no new command, so the
`make socle` check that CI covers `make qualite` and calls nothing else still
holds. The `zones` job installs the pinned toolchain to run `cargo metadata`.

**Server suites.** `SERVER_PACKAGES`, next to the `test-serveurs` target, names
the packages its nextest filter reaches (`oxyn-driver-postgres`,
`oxyn-driver-mysql`, `oxyn-driver-clickhouse`, `oxyn-exec`, `oxyn-desktop`);
a test of `script/test_zones_ci.py` keeps the list and the filter equal.
`script/zones-ci` reads that list and writes `serveurs=false` when the selection
holds none of them; the `serveurs` job then does not run. When it runs, only
the selected suites are built and run: a ClickHouse pull request no longer
replays the PostgreSQL and MySQL suites.

The aggregate `qualite` job and its `if: always()` logic do not change: a
skipped job still counts as successful. A pull request that touches only the
front end or documentation keeps ADR-0045's behaviour.

Replayed on the merge commits of `main` on 2026-10-24 (`git log
--first-parent origin/main`):

| Pull request | What it changed | Selection |
|---|---|---|
| #294 | `oxyn-driver-postgres` | `oxyn-driver-postgres`, `oxyn-exec`, `oxyn-desktop` |
| #302 | `oxyn-ai`, `oxyn-desktop`, front end | `oxyn-ai`, `oxyn-desktop` |
| #332 | `oxyn-driver`, `oxyn-exec` | 8 of 17 packages: the drivers, `oxyn-exec`, `oxyn-plugin`, `oxyn-desktop` |
| #299 | the ClickHouse driver, `oxyn-core`, `Cargo.toml`, `.config/nextest.toml` | all |
| #328 | `qualite.yml` | all |
| #329, #331 | front end only | none: `make rust` does not run (ADR-0045) |

## Consequences

* **+** a pull request on a leaf driver checks two packages — the driver and
  `oxyn-desktop` — instead of seventeen, and replays only its own server suite.
* **+** every doubt resolves to the whole workspace: a missing graph, an
  unattributed file and a lost output all give an empty value, and empty is
  everything.
* **+** locally nothing changes: without `OXYN_PACKAGES`, `make qualite` is the
  same gate, and `make socle` still verifies that CI covers it.
* **−** `-p` unifies the features of the selected packages only. A package that
  compiles inside the workspace thanks to a feature another member enables can
  fail alone on its pull request — a false red, which is a fix to make anyway.
  The reverse, a combination that only fails under workspace-wide unification,
  is not seen before `main`.
* **−** a file a crate reads outside its own directory at run time escapes the
  attribution. On 2026-10-24 the only such reads are `oxyn-desktop`'s, of
  `apps/desktop/`, and they are attributed explicitly; a new one must be added
  to `script/zones-ci`, or it is caught on `main` only.
* **−** `oxyn-desktop` depends on every crate, so almost every Rust pull
  request still runs the `serveurs` job, with at least its cancellation suite:
  the gain there is in the suites skipped, not in the job.
* **−** the `zones` job installs the toolchain on every pull request, documentation
  ones included: 10 s on the first run of 2026-10-24 (run 40065032744), on a
  job that used to take a few seconds.

**Exit cost:** low. Removing the `rust_packages` and `serveurs` outputs, the
two `OXYN_PACKAGES` environment entries of the workflow and the variable in the
`Makefile` restores ADR-0045's behaviour; the package selection of
`script/zones-ci` and its test are deleted. Nothing outside CI depends on it.

**Reconsider if** `main` turns red after a green pull request because of a
package this selection skipped more than once a month — then widen the
attribution, or go back to the whole workspace for the Rust area.

## Rejected alternatives

| Alternative | Reason for rejection |
|---|---|
| The resolved graph of `cargo metadata` (without `--no-deps`) | it needs the registry index and the sources of every dependency, on a five-minute job, for edges between members that the declared dependencies already give — and give more broadly, optional and foreign-platform ones included |
| Parsing the `Cargo.toml` files in Python | a second implementation of `workspace = true`, renames and target tables, which drifts from Cargo silently |
| Narrowing `format` and `deny` as well | `cargo fmt --all` takes seconds, and `cargo deny` reasons about the whole dependency graph, not about a package |
| Running only the changed packages, without their dependents | a changed trait breaks its implementations in other crates; that is exactly the failure a pull request must show |
| A third-party "affected crates" tool | one more pinned binary in CI for a computation of about forty lines over `cargo metadata` |
| Also narrowing `main` | removes the only run that checks everything together; ADR-0045's safety net is the reason a selection on pull requests is acceptable |
