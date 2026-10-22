# ADR-0058 — The updater key signs in a job that builds nothing

**Status:** proposed · **Date:** 2026-10-22

**Refines:** [ADR-0051](0051-automatic-updates-from-github-releases.md), on
three points: where the placeholder public key is refused (§ 2), where and
with which program the updater artifacts are signed (§ 3), and its negative
consequence "The signing step still trusts the Tauri CLI". The rest of
ADR-0051 — the updater in Rust, the endpoint, `requireSignedVersion`, the
manifest written last and verified with `minisign -V` — remains in force.

## Context

ADR-0051 § 3 kept the updater key out of `tauri build`, but not out of the
job that runs it. On `origin/main` at `377d2f1`, the `paquets` job of
`.github/workflows/livraison.yml` named the `release` environment, held a
token with `contents: write`, ran `pnpm install` and `tauri build`, then
handed the key to a later step of the **same job**, `script/livraison signer`,
which ran the `@tauri-apps/cli` binary from `apps/desktop/node_modules`.
Scoping the secret to that step isolated nothing: the build runs every
`build.rs` and proc-macro of the 1,200 packages of `Cargo.lock` (counted on
2026-10-22), Vite and its plugins, and the install scripts of the npm graph,
and any of them can rewrite `node_modules/.bin/tauri` or `script/livraison` —
files the later step then runs with the key. One compromised dependency
version was enough to read the key that every installed Oxyn trusts, and that
key has no revocation ([SECURITY](../SECURITY.md#the-updater-signing-key)).
The audit of 2026-10-21 reported it as finding F03, issue #248.

What bounds the fix, checked on 2026-10-22
([RESEARCH-NOTES](../RESEARCH-NOTES.md#ci-and-github-delivery)):

* Tauri publishes the CLI as a release binary on GitHub: the release
  `tauri-cli-v2.12.1` carries `cargo-tauri-x86_64-unknown-linux-gnu.tgz`,
  whose SHA-256 equals the digest GitHub lists for the asset, built from the
  same tauri commit as the npm package 2.12.1 of `apps/desktop/package.json`.
  Its `signer sign --app-version` writes the trusted comment
  `requireSignedVersion` checks.
* `actions/upload-artifact` v7.0.2 and `actions/download-artifact` v8.0.2
  carry files between jobs of one run; the upload stores each file relative to
  the path before the first wildcard.
* The macOS signing and notarization run **inside** Tauri's bundling, with the
  Apple credentials in the build's environment: they cannot leave the build
  without redoing the bundler's work by hand
  ([SECURITY](../SECURITY.md#the-apple-release-credentials)).

## Decision

### 1. Five jobs, and the key in the one that builds nothing

| Job | Needs | Environment | Secrets | Token | Runs build code |
|---|---|---|---|---|---|
| `brouillon` | — | — | — | `contents: write` | no |
| `paquets` (macOS, Linux x86_64, Linux aarch64) | `brouillon` | — | Apple credentials, macOS leg only | `contents: read` | **yes** |
| `signature` | `paquets` | `release` | `TAURI_SIGNING_PRIVATE_KEY`, `TAURI_SIGNING_PRIVATE_KEY_PASSWORD` | `contents: read` | no |
| `depot` | `signature` | — | — | `contents: write` | no |
| `manifeste` | `depot` | — | — | `contents: write` | no |

The workflow grants no token permission by default (`permissions: {}`).
Only `signature` names `release`, so only it can read the updater key, and
its token cannot write the release. It starts from a fresh checkout of the
tag, installs neither Node, pnpm nor Rust, and runs two programs on the key:
`script/livraison` from that checkout, and the Tauri CLI release binary of
§ 3.

### 2. The packages cross as artifacts, as data

Each `paquets` leg uploads its unsigned packages as `paquets-<cible>` —
`macos-aarch64`, `linux-x86_64`, `linux-aarch64`. `signature` downloads them
by exact name, signs the macOS archive and the two AppImages, and uploads the
three `.sig` files as `signatures`, under their platform's directory. `depot`
downloads the packages and the signatures into the same tree, then uploads
them to the draft with `script/livraison deposer`. The packages are signed,
never run, in `signature`.

### 3. The signing program is a release binary pinned by checksum

`signature` downloads `cargo-tauri` from Tauri's GitHub release of the CLI
version in `apps/desktop/package.json`, checks the archive's SHA-256 —
written in the workflow and in
[RESEARCH-NOTES](../RESEARCH-NOTES.md#ci-and-github-delivery) — before
unpacking it, and names it in `$TAURI`. `script/livraison signer` has no
default CLI: it refuses to run without `$TAURI`, or with a path under
`node_modules`.

### 4. The placeholder public key is refused before the build

`brouillon` refuses the placeholder `REPLACE_WITH_UPDATER_PUBLIC_KEY`
(`script/livraison brouillon`), before the draft exists and before an hour
of build. `script/livraison cle`, the first step of `signature` to hold the
key, refuses an empty secret and the placeholder again before downloading
anything; `manifeste` still refuses it before writing `latest.json`.

### 5. Held by a test of the workflow file

The workflow only runs on a `v*` tag, so no pull request exercises it.
`script/test_livraison_workflow.py`, run by `make socle`, reads
`livraison.yml` and refuses a change to the table of § 1 — the updater
secrets or `release` in another job, a build command or a toolchain action
in `signature`, `contents: write` or `GH_TOKEN` outside the three writing
jobs — as well as an artifact name or path that diverges between producer and
consumer, and a checksum check placed after the unpacking or absent from
RESEARCH-NOTES.

## Consequences

* **+** No job that runs build code holds the updater key, and no job that
  holds it can write the release.
* **+** The signing program comes from bytes pinned in the repository, not
  from a directory the build writes.
* **+** A regression of the boundary fails `make socle` on the pull request
  that introduces it, not at the next release.
* **−** **The isolation protects the key, not the packages.** A compromised
  build still produces a compromised package, and `signature` signs it.
* **−** **The Apple credentials stay in the macOS build**, readable by its
  code: an accepted risk, bounded and argued in
  [SECURITY](../SECURITY.md#the-apple-release-credentials).
* **−** **The signing job trusts Tauri's GitHub release.** A malicious asset
  published under that tag before the checksum was recorded would have been
  pinned with it; the checksum only guarantees the bytes stay the ones
  checked on 2026-10-22.
* **−** **The CLI version is pinned twice**: in `apps/desktop/package.json`
  for the build, and in the workflow's URL and checksum for the signing. A
  bump of one without the other is caught by no test; both rows of
  RESEARCH-NOTES must move together.
* **−** **The topology is first executed by a release.** The test of § 5
  reads the file; it does not run the actions. The real artifacts, paths and
  signatures are proven by the first tag after this ADR.
* **−** The run gains two jobs and three artifact round trips: a few minutes
  on a release that already takes about an hour.

**Exit cost:** low. One workflow file, the `signer` and `cle` commands of
`script/livraison` and one test file; merging the jobs back is an afternoon.
Nothing in the application depends on how the signatures were produced, only
on the key that made them.

**Reconsider if** the Tauri bundler can sign and notarize a macOS bundle in an
invocation separate from the build (the Apple credentials could then leave
`paquets` too); if the updater key moves to a signing service that never
hands it to a runner; if Tauri stops publishing CLI release binaries, or the
plugin stops checking minisign signatures; if GitHub offers secrets scoped to
a step whose isolation from the job's earlier steps it documents.

## Rejected alternatives

| Alternative | Reason for rejection |
|---|---|
| Keep signing in `paquets`, the key scoped to the signing step (ADR-0051 § 3) | The earlier steps of the job wrote `node_modules` and could rewrite the scripts and the CLI that step runs: the scoping protected nothing from the build |
| `pnpm install` in the signing job, to run the CLI the lockfile pins | Runs the install scripts of the whole npm graph in the job that holds the key |
| `cargo install tauri-cli --locked` in the signing job | A Rust build — hundreds of `build.rs` and proc-macros — in the job that holds the key, the exposure this ADR removes |
| Signing with `minisign -S` directly | The `.sig` format, the encoding of Tauri's private key and the trusted comment `requireSignedVersion` reads are Tauri's: reproducing them by hand is a second implementation of the contract every installed copy checks, tested against nothing |
| Moving the macOS Developer ID signing and notarization into `signature` | Means redoing outside Tauri the bundler's nested signing, hardened runtime, notarization, stapling and DMG, in a macOS job only a real release could exercise; the accepted risk is in [SECURITY](../SECURITY.md#the-apple-release-credentials) |
| Moving the Apple credentials into `release` | The macOS build would have to name `release`, and one line would then hand it the updater key |
