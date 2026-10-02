# GitHub releases

GitHub Actions builds the packages; the maintainer publishes the resulting
draft after review. `.github/workflows/livraison.yml` runs on a pushed `v*`
tag, or manually with an existing tag. The tag must equal `v` followed by
`crates/oxyn-desktop/tauri.conf.json`'s version. Both jobs check out that exact
tag, including for a manual run selected from another branch.

`macos-latest` produces an Apple Silicon DMG; `ubuntu-24.04` produces DEB,
RPM and AppImage packages. Intel macOS and Windows are not in this matrix.
`make desktop PROFIL=release` is the build entry point. The workflow installs
the pinned `cargo-about` binary after verifying its checksum so that required
third-party notices are included.

The workflow also builds what installed copies update from
([ADR-0051](adr/0051-automatic-updates-from-github-releases.md)): after the
build, a step of its own signs the updater artifacts —
`Oxyn_<version>_aarch64.app.tar.gz` and the AppImage, each with its `.sig` —
and a final `manifeste` job verifies them and writes `latest.json`.

## Apple setup

The macOS job requires all seven **repository Actions secrets** below. Missing
values stop it before dependency installation; it never falls back to unsigned
delivery. Linux does not receive Apple secrets.

| Secret | Value |
|---|---|
| `APPLE_CERTIFICATE` | Base64 of a password-protected `.p12` export containing the Developer ID Application certificate **and its matching private key** |
| `APPLE_CERTIFICATE_PASSWORD` | Password chosen for that `.p12` export |
| `APPLE_SIGNING_IDENTITY` | Full identity: `Developer ID Application: <name> (<team ID>)` |
| `APPLE_API_KEY` | Key ID of the dedicated App Store Connect team API key |
| `APPLE_API_ISSUER` | Issuer ID shown above the team keys table |
| `APPLE_API_PRIVATE_KEY` | Contents of the downloaded `.p8` private key |
| `APPLE_TEAM_ID` | The ten-character membership team ID |

Use [repository Actions secrets](https://github.com/so-keyldzn/oxyn/settings/secrets/actions).
Enter credentials yourself, with agent screen inspection paused. Never put
them in chat, repository files, command arguments, logs or the clipboard.
Do not export a private key into the worktree.

In Keychain Access, export the matching identity from **My Certificates** as
a password-protected `.p12`. If its private key is absent, downloading the
public `.cer` from Apple does not recover it: use the Mac that created the
certificate, or create a new Developer ID Application certificate from a new
CSR. Do not revoke an existing certificate to make room without checking its
other users. No provisioning profile is required for this distribution path.

With a GitHub CLI session authorized to manage this repository's secrets, the
maintainer can send the Base64 directly to GitHub without a clipboard or
intermediate text file. Run this personally, replacing only the file path:

```sh
set -o pipefail
openssl base64 -A -in /absolute/path/outside-the-repo/developer-id.p12 |
  gh secret set APPLE_CERTIFICATE --repo so-keyldzn/oxyn
gh secret set APPLE_CERTIFICATE_PASSWORD --repo so-keyldzn/oxyn
gh secret set APPLE_API_PRIVATE_KEY --repo so-keyldzn/oxyn < /absolute/path/outside-the-repo/AuthKey.p8
```

The export password command prompts privately; the API key is read directly from its file. A `403` listing or setting secrets
means the CLI credential lacks access; it does not prove that secrets are
absent. Use the browser or a separately authorized CLI session; do not paste
a GitHub token into a chat to repair access.

Create the team API key in App Store Connect → Users and Access → Integrations,
with Developer access. Download its private key once and transfer it directly
to GitHub. Team keys apply across the account’s apps; keep this key dedicated
to release automation and revoke it when replacing it.

`script/apple-release build` writes the API key to a mode-0600 file in a
private temporary directory outside the workspace, passes only its path to
Tauri and removes it on normal exit, including a build failure. The raw API
key is removed from the child environment.
Tauri imports the certificate into a temporary keychain, signs with hardened
runtime, submits the app to Apple and staples the accepted ticket before
creating the signed DMG. Its temporary keychain is deleted on normal cleanup;
these jobs use disposable GitHub-hosted runners, never a persistent runner.
`script/apple-release verify` then checks both signatures and the configured
team, the application's hardened runtime, stapled ticket and Gatekeeper
assessment. A failed check prevents the macOS upload. The DMG itself is signed;
the notarized, stapled object inside it is the `.app`. The updater archive is
made by `script/apple-release build` from that same `.app`, after stapling,
the way the bundler would make it; `verify` extracts it and runs the same
`codesign`, `stapler` and `spctl` checks on the `.app` it contains.

## Updater signing key

Every installed Oxyn accepts an update only if its archive carries a minisign
signature from the key whose public half is compiled in
(`plugins.updater.pubkey` in `crates/oxyn-desktop/tauri.conf.json`). GitHub
never holds the private key in clear: it lives in two secrets of the `release`
environment and in two offline backups, nowhere else
([I-03](../CLAUDE.md#i-03)). Generating it, storing it and rotating it are the
maintainer's alone; none of it is delegated to an agent.

| Secret of the `release` environment | Value |
|---|---|
| `TAURI_SIGNING_PRIVATE_KEY` | Contents of the private key file written by `tauri signer generate` |
| `TAURI_SIGNING_PRIVATE_KEY_PASSWORD` | The password chosen at generation |

**Which steps hold the key.** Only two steps of the package jobs: the key
check, `script/livraison cle`, which refuses an empty secret or the
placeholder public key before an hour of build and names the variables,
never their values; and the signing step, `script/livraison signer`, which
runs the pinned `tauri signer sign --app-version <version>` on the built
artifact and nothing else. The build itself — `tauri build`, every `build.rs`
and proc-macro of `Cargo.lock`, Vite and its plugins — runs without it:
any of them could read its environment, and one compromised dependency
version would be enough to take the key. That is why no build produces
signed updater artifacts (`createUpdaterArtifacts` stays off): the macOS
archive is made by `script/apple-release build` from the stapled `.app`, and
the AppImage is signed as the bundler wrote it.

**The `release` environment.** Repository secrets are readable by any
workflow run on any branch: a collaborator, or a token with the `workflow`
scope, could push a workflow that prints them. Environment secrets reach only
the jobs that name the environment, and the environment's deployment rule
decides which refs may run them. Before the first release, the maintainer
creates it in **Settings → Environments → New environment**:

1. Name: `release`.
2. **Deployment branches and tags → Selected branches and tags → Add rule →
   Tag**, pattern `v*`. No branch rule: a run from `main` or any branch then
   cannot reach the secrets, including a manual run.
3. **Required reviewers**: the maintainer. Each release then waits for an
   approval before the package jobs read the key — available because the
   repository is public.
4. Add the two secrets above to the environment (commands below), then
   delete any repository secret of the same name.
5. Protect the `v*` tags with a tag ruleset (**Settings → Rules → Rulesets →
   New tag ruleset**, target `v*`, restrict creation, update and deletion to
   the maintainer): otherwise whoever can push a tag can still start a run
   the environment admits.

The package job is the only one that names `release`; `brouillon` and
`manifeste` hold no secret. GitHub creates an environment that a job names
and that does not exist yet, **without any rule**: create it with its rules
first. Moving the seven Apple secrets into the same environment is advised
for the same reason; the workflow reads them identically.

**Why the verification.** The Tauri CLI only *warns* when the private key
does not match the committed public key, and a rerun can leave in the draft
the archive of one run beside the signature of another. Either way every
installed Oxyn would refuse the update as a signature failure — shown as
such, every day, indistinguishable from an attack — until the next release.
So the `manifeste` job downloads both archives from the draft and verifies
each with `minisign -V` against the public key of `tauri.conf.json` before
it reads the signed version or writes anything. The verifier is the
reference implementation, minisign 0.12, installed from its GitHub release
archive pinned by SHA-256
([RESEARCH-NOTES](RESEARCH-NOTES.md#tauri-updater-contracts--checked-on-2026-10-02)),
like `cargo-about`; it verifies the `.sig` Tauri writes once decoded from
Base64. It runs in `manifeste` rather than in the package jobs because there
it checks the bytes users will download, on one platform, with one pinned
binary. Python's standard library has no Ed25519, and the repository has no
precedent of a vendored implementation.

**Generate**, once, outside the repository, with a password:

```sh
pnpm --dir apps/desktop tauri signer generate -w /absolute/path/outside-the-repo/oxyn-updater.key
```

The command prompts for the password, writes the private key to that path and
the public key next to it, with `.pub` appended. Then:

1. Paste the **public** key (`oxyn-updater.key.pub`, one line of Base64) into
   `plugins.updater.pubkey` and commit it through a pull request.
2. Send both secrets to the `release` environment from the file and from a
   prompt, never through the clipboard, a chat or a command argument:

   ```sh
   gh secret set TAURI_SIGNING_PRIVATE_KEY --env release --repo so-keyldzn/oxyn < /absolute/path/outside-the-repo/oxyn-updater.key
   gh secret set TAURI_SIGNING_PRIVATE_KEY_PASSWORD --env release --repo so-keyldzn/oxyn
   ```

3. Copy the private key file to **two offline backups** on different media,
   with the password stored apart from them (a password manager), then delete
   the working copy. A GitHub secret cannot be read back: if both backups are
   lost, the key is lost.

**Rotate** while the old key still exists. Release N, signed with the old key,
carries the new public key; release N+1 is signed with the new key. Users who
skip N stay on the old key: they get N+1 only after updating to N, so keep N
published. Only then replace the two secrets and the backups.

**Loss.** Without the private key, no installed copy can be updated again.
Generate a new key, publish a release signed with it, and announce that every
user must download it by hand once.

**Compromise.** Whoever holds the key and the password can sign an archive
every installed copy will install, if they can also serve a manifest. Rotate
at once if the old key is still in hand; otherwise treat it as a loss. Remove
the secrets, and audit the published releases' `.sig` against the archives.

## Launch and publish

1. Merge the reviewed workflow and scripts into `main`. Run `make qualite`
   on the intended source and review the matching GitHub quality run.
2. Set the release version consistently in the workspace and Tauri config,
   commit the intended source, then push the matching version tag. A tag push
   triggers delivery automatically; ordinary branch pushes do not create releases.
3. Alternatively, select **Actions → livraison → Run workflow**, choose the
   version tag itself under **Use workflow from**, and enter that same tag,
   or run:

   ```sh
   gh workflow run livraison.yml --repo so-keyldzn/oxyn --ref v0.0.1 -f tag=v0.0.1
   ```

   The manual workflow must first exist on the default branch. Started from
   a branch, the run is refused by the `release` environment's tag rule. The
   example version must be replaced when the configured version changes.
4. Wait for both package jobs, inspect their checks and download/install the
   packages from the draft.
5. Wait for the `manifeste` job. It runs once both package jobs are green,
   with no secret but `GH_TOKEN`: it requires exactly one
   `*_aarch64.app.tar.gz` and one `*.AppImage` in the draft, each with its
   signature, downloads them, verifies each archive against its signature
   and the committed public key, checks the signed version, and uploads
   `latest.json` with the keys `darwin-aarch64` and `linux-x86_64-appimage`.
   It is the only job that uploads `latest.json`, and it refuses a draft that
   already holds one. A signature that does not verify means a key pair that
   does not match, or an archive and a signature from two runs: delete both
   from the draft and rerun the package job of that platform.
6. **Publish the draft only when both package jobs and `manifeste` are green
   and `latest.json` is attached.** Publishing is what offers the update:
   `https://github.com/so-keyldzn/oxyn/releases/latest/download/latest.json`
   serves the latest published, non-prerelease release only. A release
   published without `latest.json` makes every installed copy's check fail
   until the next one.

Reruns never replace assets. Remove a specific asset from a **draft** manually
before rebuilding it. A published release is refused before creation/upload,
and its status is checked around each file upload. If publication races an
upload, that one file may arrive; the workflow stops without removing it.
Keep the release in draft until every job has finished.

## Verification boundaries

`make socle` runs the simulated GitHub release tests and Apple refusal tests;
it does not contact Apple or prove a signature. The first successful signed
GitHub build, ticket verification and installation on a clean Mac are the
end-to-end evidence. Sources and checked tool contracts are recorded in
[RESEARCH-NOTES](RESEARCH-NOTES.md#apple-release-contracts--checked-on-2026-10-01).

The same tests cover `manifeste`, `cle` and `signer`, with a simulated
minisign and Tauri CLI; the real pair — `tauri signer sign` 2.12.1, then
`minisign -V` 0.12 accepting the result and refusing a modified archive or
another key — was checked by hand on 2026-10-02. They do not prove that an
installed Oxyn accepts the result. That takes two steps:

1. **Local rehearsal**, with the `update-rehearsal` cargo feature, never
   enabled in CI: a throwaway key, bundles 0.0.1 and 0.0.2 served on
   `127.0.0.1`. Check the download, the install on ⌘Q, the install on the
   Dock's Quit, "Restart now", the absence of a recovery screen after the
   relaunch, a tampered `.sig` ending in a signature error, and an offline
   check failing silently.
2. **Real releases.** Publish v0.0.2, installed by hand (v0.0.1 has no
   updater), then v0.0.3. Check that v0.0.2 updates itself on a clean Mac and
   as an AppImage, and that a `.deb` install says "Updates are managed by your
   package manager".

Updater contracts are recorded in
[RESEARCH-NOTES](RESEARCH-NOTES.md#tauri-updater-contracts--checked-on-2026-10-02).
