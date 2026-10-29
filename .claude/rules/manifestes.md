---
paths:
  - "**/Cargo.toml"
  - "Cargo.lock"
  - "rust-toolchain.toml"
  - "Makefile"
  - "deny.toml"
  - "clippy.toml"
  - ".cargo/config.toml"
  - ".config/nextest.toml"
  - "renovate.json5"
  - ".github/workflows/*.yml"
  - "script/*"
---

# Manifests and tooling — conventions

## No version is written from memory

[I-12](../../CLAUDE.md#i-12). A plausible and wrong version shows up neither at
compile time, nor in tests, nor in review — it shows up when someone tries to
build the project six months later.

Before adding or changing a dependency: [`/versions`](../commands/versions.md).
The chosen value is recorded in
[RESEARCH-NOTES](../../docs/RESEARCH-NOTES.md) **in the same commit**.

`make versions` refuses a version pinned in `Cargo.toml` or in
`apps/desktop/package.json` that the notes do not record, so a bump pull request,
Dependabot's included, carries its dated row in the same commit.
`renovate.json5` is a dormant configuration that enforces nothing.

## A declaration that tracks the defaults carries its version

`default-features = false` with an explicit list usually means a minimal set: a
feature the crate adds upstream is meant to stay off, and nothing has to follow
it. The opposite case exists — the point of the declaration is the crate's
defaults **minus** a few, `wasmtime` without `cache` — and there an added
default feature stays off silently, against the intent. That intent is written
where the declaration is, in the root `Cargo.toml`:

```toml
[workspace.metadata.oxyn.feature-contracts]
wasmtime = { version = "48.0.7", dropped = ["cache"] }
```

`version` is the release whose `default` array was read, `dropped` the features
left out on purpose, and the array itself is recorded in
[RESEARCH-NOTES](../../docs/RESEARCH-NOTES.md) with its date. `make versions`
then refuses a row whose `version` is not what `Cargo.lock` resolves: a bump is
the moment to read the array again, and
[`/versions`](../commands/versions.md) compares the declared list with the
registry and names what is missing from it. A declaration with no row is
checked by nobody — the row is the only place that intent is written.

## The workspace centralizes

Shared dependencies go in `[workspace.dependencies]`, crates refer to them with
`{ workspace = true }`. Two versions of the same crate in the graph means twice
the compiled code and types incompatible with each other — the resulting error
message is famous for its opacity.

## `rust-toolchain.toml`

**Exact** version, with `rustfmt` and `clippy`
([ADR-0008](../../docs/adr/0008-chaine-outils-rust.md)). A version bump is a
deliberate commit, which updates
[RESEARCH-NOTES](../../docs/RESEARCH-NOTES.md) at the same time.

## New dependency

It is justified in review: what it brings, and the cost of doing without it
([SECURITY](../../docs/SECURITY.md#dependencies)). A crate used in one place for
one function is a candidate for rewriting, not a given. An unmaintained crate
on an external boundary is a risk to document.

## `make qualite`

It is the quality gate, and the only entry point. If a check is not in it, it
does not run: CI calls it
([.github/workflows/qualite.yml](../../.github/workflows/qualite.yml)), the
`Stop` hook reminds of it, the definition of "done" relies on it. Adding a check
elsewhere makes it optional.

**CI adds no check.** It calls the targets of `make qualite`, split into
parallel jobs, and nothing else. A check that only existed in the workflow file
would be irreproducible locally: one would discover it exists by seeing it fail.

The split carries the opposite risk: a target added to `qualite` and forgotten
in the workflow would never run in CI. `make socle` refuses it
(`check_ci_coverage`). A new target of the gate is therefore added to both
places in the same commit.

On a pull request, the jobs of an untouched area are skipped
([ADR-0045](../../docs/adr/0045-ci-selective-sur-les-pull-requests.md)); on
`main`, everything runs, unless the commit only touches documentation. A new job that calls `make` is added to the `needs` of
the aggregate `qualite` job (`check_ci_aggregate`), and a new top-level
directory is classified in `script/zones-ci` — otherwise it triggers everything.

## Where the mechanizable prohibitions live

| File | What it refuses |
|---|---|
| [clippy.toml](../../clippy.toml) | forbidden call paths — `disallowed-methods`, with the reason and the replacement |
| [.cargo/config.toml](../../.cargo/config.toml) | nothing; it imposes the flags that must hold for everyone, including the macOS target |
| [.config/nextest.toml](../../.config/nextest.toml) | a hanging test, and two tests sharing a server |
| [verifier_versions.py](../hooks/verifier_versions.py), through `make versions` | a version pinned in `Cargo.toml` or `apps/desktop/package.json` that [RESEARCH-NOTES](../../docs/RESEARCH-NOTES.md) does not record, or records differently, and a feature contract whose `version` is no longer the one `Cargo.lock` resolves — offline, [I-12](../../CLAUDE.md#i-12) |

An invariant that boils down to a call path belongs in `clippy.toml`, not in a
review. `clippy.toml` holds for the **whole** workspace: a prohibition that must
only hold for one crate has no place there.

## A crate is not created by hand

`script/nouvelle-crate <name> "<description>"`. The reason is written in
[CLAUDE.md](../../CLAUDE.md): a `paths:` rule loads when a file is **read**, not
when it is created. A manifest written from memory forgets
`[lints] workspace = true`, and the crate then escapes all the repository's
lints without anything failing. `make socle` catches it afterwards; the script
prevents it.
