# ADR-0008 — Rust toolchain pinned in the repository

**Status:** accepted · **Date:** 2026-09-05

## Context

Fact checked on 2026-09-05 ([RESEARCH-NOTES](../RESEARCH-NOTES.md#rust-toolchain)):

- Rust stable is at `1.98.1`;
- the maintainer's development machine is at `1.89.0`, that is **nine minor
  versions behind**;
- `gpui 0.2.2` uses **edition 2024**, which requires at least Rust `1.85`;
- `gpui` declares **no MSRV** (`rust_version` absent from its metadata), so
  nothing guarantees that a future version will remain compatible with an old
  toolchain.

`1.89.0` compiles edition 2024. But without pinning, the development machine and
continuous integration use two different compilers. The failure mode is silent
in the most expensive direction: the code compiles locally, and `clippy` in CI
reports diagnostics added between 1.89 and 1.98 that the developer has never
seen. Conversely, code using an API stabilized after 1.89 passes CI and does not
compile on their machine.

## Decision

A `rust-toolchain.toml` file at the root pins an **exact** version of the
toolchain, with the `rustfmt` and `clippy` components.

The pinned version is the same everywhere: development machine, continuous
integration, release. An upgrade is a deliberate change, with its own commit,
and updates [RESEARCH-NOTES](../RESEARCH-NOTES.md#rust-toolchain) in the
same commit.

The initial value is to be set at the first code commit. The choice is between
two options, and it is documented here:

- `1.98.1`, today's stable: requires the maintainer to update their machine;
- `1.89.0`, the existing toolchain: freezes the project on a year-old compiler,
  with no benefit.

The recommendation is `1.98.1` — a new project has no reason to be born with a
year of tooling debt.

### Chosen value: `1.98.1`

Set on 2026-09-05, at the first compilation of the workspace. The `1.89.0` that
appeared in `rust-toolchain.toml` was the value this ADR rules out: the file had
been written before the decision was made, and it therefore contradicted the
document meant to ground it.

The first `cargo check --workspace` settled the matter beyond appeal:

```
error: rustc 1.89.0 is not supported by the following packages:
  sqlx@0.9.0 requires rustc 1.94.0
```

The floor is not `1.94.0` but **`1.95.0`**, imposed by `wasmtime 48.0.1`
([RESEARCH-NOTES](../RESEARCH-NOTES.md#msrvs-imposed-by-dependencies)). It
does not show today, because `wasmtime` sits behind the `wasm-host` feature of
`oxyn-plugin`, disabled by default: the default build does not compile it, so
`cargo` does not check its `rust-version`. This is the failure mode this ADR
describes — silent until the day someone enables the feature, and
incomprehensible at that moment.

Hence two distinct values, not one:

| File | Value | What it expresses |
|---|---|---|
| `rust-toolchain.toml` | `1.98.1` | the compiler **used**, identical everywhere |
| `Cargo.toml`, `rust-version` | `1.95` | the minimum **supported**, `wasm-host` included |

The two are complementary, as the table of rejected alternatives below already
says.

## Consequences

* **+** A single set of `clippy` diagnostics: the quality gate means the same thing
  on the workstation and in CI.
* **+** The edition and the MSRV stop being implicit.
* **+** `rustup` installs the right version on the first `cargo` run in the
  repository: no procedure to document.
* **−** A contributor who is offline, or behind a restricted mirror, must have the
  pinned version.
* **−** Pinning needs upkeep: a file forgotten for two years is a debt that grows on
  its own.

**Exit cost:** none, deleting the file is enough. **Reconsider if** the project
welcomes contributors whose distribution imposes a system toolchain: the exact
pin should then become a minimum.

## Rejected alternatives

| Alternative | Reason for rejection |
|---|---|
| Pin nothing | this is the current state, and it produces exactly the silent divergence described above |
| Pin `stable` without a version | freezes nothing: "stable" means a different compiler every week |
| Only declare a `rust-version` in `Cargo.toml` | expresses a minimum, does not impose the version used; the two are complementary, not substitutable |
