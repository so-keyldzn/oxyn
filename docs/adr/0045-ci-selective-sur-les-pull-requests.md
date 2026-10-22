# ADR-0045 — On a pull request, CI skips the jobs whose area is not touched; on `main`, everything runs

**Status:** accepted · **Date:** 2026-09-25

**Clarifies:** the rule of [CLAUDE.md](../../CLAUDE.md#what-is-executed) that
CI calls `make qualite` in parallel jobs, adding nothing, and that `make socle`
verifies it misses no target. This rule remains true; it stops saying that
**every** push runs **all** targets.

## Context

Every pull request currently launches all the jobs of
[qualite.yml](../../.github/workflows/qualite.yml): `controles`, two slices of
`stories`, and `rust`, whose timeout is capped at 75 minutes. A PR that only
touches a document recompiles the workspace; a PR that only touches the front
end recompiles all the Rust, and a Rust PR replays the stories in Chromium.

Two costs follow, observed on 2026-09-24 and 2026-09-25:

* the account's minutes quota was exhausted on 2026-09-24 (the macOS matrix
  was removed from PRs for that reason, see the workflow comment);
* agents working in parallel wait for the `rust` job on a change that does not
  touch it, and the PR flow is slowed down.

The user settled it on 2026-09-25: lighten the local machine and speed up the
agents' flow, **without over-engineering**.

## Decision

**On the `pull_request` event only**, a first `zones` job runs
[`script/zones-ci`](../../script/zones-ci), which compares the PR's merge
commit to its first parent (`git diff --name-only HEAD^1 HEAD`, `fetch-depth: 2`)
and puts each file in an area:

| File | Area |
|---|---|
| `Makefile`, `.github/`, `Cargo.toml` and `Cargo.lock` at the root, `.cargo/`, `.config/`, `rust-toolchain.toml`, `clippy.toml`, `deny.toml`, `renovate.json5`, `.claude/`, `.agents/`, `script/` | **cross-cutting**: all areas |
| `apps/desktop/` | `front` |
| `crates/`, `drivers/` | `rust` |
| `docs/`, any other `*.md` | `docs` |
| any other file | **cross-cutting**: a file the script cannot classify triggers everything |

The script uses no external action: `git` and Python, already present on the
runner.

The jobs depend on it as follows:

| Job | Runs on a PR if | What it skips otherwise |
|---|---|---|
| `controles` | always | `make front-controles` and the Node installation, if `front` is not touched; `make socle todo` always runs |
| `stories` | `front` | the whole job |
| `rust` | `rust` or `front` | `make rust` and its tooling if only `front` is touched; `make front-build` runs as soon as either is |

`rust` runs for a front-end PR because it carries `front-build`:
`oxyn-desktop` embeds `apps/desktop/dist` at compile time, and the front-end
build has no other job. The `docs` area drives no job: `controles`, which
checks the links and the ADR index, always runs. It is displayed in the
`zones` job's log, so that one can read what was decided.

A final **`qualite`** job depends on all the others (`if: always()`), and
succeeds if and only if `zones` succeeded and no other job failed or was
cancelled — a `skipped` job counts as successful. **This job, and it alone, is
what branch protection will require**: a required status check on a skipped
job would stay pending forever. On 2026-09-25, branch protection is still
refused to this private repository
([RESEARCH-NOTES](../RESEARCH-NOTES.md#ci-and-github-delivery)): until then,
`qualite` is the only check to read before merging.

**Amendment (2026-10-03).** The repository is now public. GitHub ruleset
[`24429281`](https://github.com/so-keyldzn/oxyn/rules/24429281) requires a
pull request (with zero required approvals) and the aggregate `qualite` status
check from GitHub Actions app `15368` on `main`; it also forbids deletion and
non-fast-forward updates. Repository administrators may bypass these rules
only when merging a pull request (`bypass_mode: pull_request`); direct pushes
to `main` are refused for everyone, including administrators.

**Amendment (2026-10-04).** At the maintainer's request, the administrator
bypass is now `bypass_mode: always`: an administrator may push directly to
`main`, so a commit whose CI is red can reach it that way. Everyone else still
needs a pull request with a green `qualite`, and the full run on `main` stays
the safety net.

**Amendment (2026-10-20).** The repository is public (`private: false`,
`visibility: public` through `gh api repos/so-keyldzn/oxyn`), and GitHub's
documentation states: "Use of the standard GitHub-hosted runners is free and
unlimited on public repositories."
([GitHub-hosted runners reference](https://docs.github.com/en/actions/reference/runners/github-hosted-runners),
read on 2026-10-20, recorded in
[RESEARCH-NOTES](../RESEARCH-NOTES.md#audit-follow-up--checked-on-2026-10-20)).
The quota exhausted on 2026-09-24, while the repository was private, is what
removed the macOS matrix from pull requests (Context above): it no longer
applies. The `controles`, `stories` and `rust` jobs therefore run their
`macos-latest` leg on pull requests again, as they do on `main`. This
supersedes the sentence below, "the macOS matrix is added as before": macOS is
no longer something only `main` adds.

Nothing else changes. The `zones` job still selects the areas, a skipped job
still counts as successful, and the aggregate `qualite` job is still the only
check the ruleset requires. The macOS legs are not required checks, so their
return renames no required check. `workflow_dispatch` stays, to run the whole
gate on a branch that has no pull request open.

**On `push` to `main` and on `workflow_dispatch`, everything always runs.**
`script/zones-ci` returns all areas outside the `pull_request` event, and the
macOS matrix is added as before. It is the safety net: what a PR wrongly
skipped is caught at merge.

**Amendment (2026-10-21).** At the maintainer's request, two narrowings for
documentation, observed on 2026-10-21: merging a documentation PR launched the
whole matrix on `main` (`rust` alone took 18 minutes on macOS and 11 on Linux),
and editing an agent's memory under `.claude/` ran `rust` and the stories on
its PR.

* **On `push` to `main`**, a `paths` filter keeps the workflow from starting
  when every modified file is under `docs/` or is a `*.md` file outside
  `apps/`, `crates/` and `drivers/`. Markdown under those three directories
  still triggers: the agent prompts of `oxyn-ai` are compiled in through
  `include_str!`. A commit that mixes documentation and anything else runs
  everything, as before. The filter stays off `pull_request`: a workflow that
  does not start leaves the required `qualite` check pending forever.
* **In `script/zones-ci`**, a `*.md` file is classified `docs` before the
  cross-cutting rule: a Markdown file of `.claude/`, `.agents/` or `.github/`
  is read, never built or executed. The code directories keep precedence over
  the `.md` rule, for the same reason as above.

What it costs: on `main`, a documentation-only commit no longer replays
`make socle`. It passed `controles` on its PR; the case left uncovered is two
PRs merged one after the other, one removing a file the other's documentation
links to. The next commit that runs the workflow on `main` reports it.

`make socle` keeps verifying that the union of the workflow's `run: make …`
covers `make qualite`. It additionally verifies that every job calling `make`
appears in the `needs` of the `qualite` job: otherwise its failure would not
block the merge.

Locally, [`make verif-rapide`](../../Makefile) applies the same idea to
day-to-day work — only check what changed since `origin/main` — without
changing what is conclusive: **`make qualite` remains the gate**.

**Amendment (2026-10-22).** `make test-serveurs` is a target of `make qualite`
and the Linux-only `serveurs` job runs it alongside `rust` when the Rust area is
touched, against disposable PostgreSQL, TLS PostgreSQL and MySQL services. In
CI, a missing nextest binary, an unset server URL or a skipped URL-dependent
suite fails the target. Outside CI, when no server URL is configured, the
target explicitly reports `server suites NOT run` and succeeds; setting any
server URL is an intentional local server-suite run and keeps the CI failure
semantics.

## Consequences

* **+** a documentation PR launches neither Rust nor stories; a front-end PR
  launches neither clippy nor the Rust tests; a Rust PR does not launch the
  stories.
* **+** the required status checks come down to a single name, `qualite`,
  stable when the matrix or the job split changes.
* **+** the rule "CI adds no check" holds: every job still calls targets of
  `make qualite`, and `make socle` still verifies it.
* **−** a PR can be green while `make qualite` would fail on the same tree: a
  Rust PR that would break a story is not seen before `main`. The case exists
  — an IPC command renamed on the Rust side without the front end following —
  and it is `main` that turns red, after the merge.
* **−** the classification of files is a second description of the
  repository, to keep up to date when a directory appears. The fallback
  ("unknown ⇒ everything") bounds the risk: an omission costs minutes, not a
  check.
* **−** the day branch protection becomes available, it must require `qualite`
  and no other job: requiring `rust` would block every PR that does not touch
  Rust, on a check that will never come.

**Exit cost:** low. Removing the `zones` job, the `if:` that read it and the
`qualite` job restores the previous workflow; `script/zones-ci` and its test
are deleted. Branch protection can keep requiring `qualite`.

**Reconsider if** `main` turns red after merging a green PR more than once a
month because of a skipped job — the classification is then too optimistic,
and one must either widen an area or go back to running everything.

## Rejected alternatives

| Alternative | Reason for rejection |
|---|---|
| `paths` / `paths-ignore` on the `pull_request` trigger | filters the whole workflow, not a job; a workflow not triggered leaves a required status check pending forever |
| A third-party path-filtering action | one more external dependency in a workflow that already pins five by digest, for a thirty-line computation |
| Compute the dependent crates and test only them in CI | the graph is hard to compute without `cargo metadata`, and CI is precisely the place where the whole workspace must pass; the real gain is in whole skipped jobs |
| Also filter on `main` | removes the only place where everything is checked together; the slightest classification error would become invisible. Narrowed on 2026-10-21 to documentation-only commits, which build nothing (amendment above) |
| Leave CI as it is and only lighten local work | answers neither the quota exhaustion nor the agents waiting on the `rust` job |
