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
([ADR-0051](adr/0051-automatic-updates-from-github-releases.md)): the package
jobs run `make desktop PROFIL=release MISE_A_JOUR=1`, which signs the updater
artifacts — `Oxyn_<version>_aarch64.app.tar.gz` and the AppImage, each with
its `.sig` — and a final `manifeste` job writes `latest.json` from them.

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
made by the bundler from that same `.app`, after stapling; `verify` extracts
it and runs the same `codesign`, `stapler` and `spctl` checks on the `.app`
it contains.

## Updater signing key

Every installed Oxyn accepts an update only if its archive carries a minisign
signature from the key whose public half is compiled in
(`plugins.updater.pubkey` in `crates/oxyn-desktop/tauri.conf.json`). GitHub
never holds the private key in clear: it lives in two Actions secrets and in
two offline backups, nowhere else ([I-03](../CLAUDE.md#i-03)). Generating it,
storing it and rotating it are the maintainer's alone; none of it is
delegated to an agent.

| Secret | Value |
|---|---|
| `TAURI_SIGNING_PRIVATE_KEY` | Contents of the private key file written by `tauri signer generate` |
| `TAURI_SIGNING_PRIVATE_KEY_PASSWORD` | The password chosen at generation |

The package jobs pass them only to the key check and the build steps.
`script/livraison cle` refuses an empty secret or the placeholder public key
before an hour of build, and names the variables, never their values.

**Generate**, once, outside the repository, with a password:

```sh
pnpm --dir apps/desktop tauri signer generate -w /absolute/path/outside-the-repo/oxyn-updater.key
```

The command prompts for the password, writes the private key to that path and
the public key next to it, with `.pub` appended. Then:

1. Paste the **public** key (`oxyn-updater.key.pub`, one line of Base64) into
   `plugins.updater.pubkey` and commit it through a pull request.
2. Send both secrets to GitHub from the file and from a prompt, never through
   the clipboard, a chat or a command argument:

   ```sh
   gh secret set TAURI_SIGNING_PRIVATE_KEY --repo so-keyldzn/oxyn < /absolute/path/outside-the-repo/oxyn-updater.key
   gh secret set TAURI_SIGNING_PRIVATE_KEY_PASSWORD --repo so-keyldzn/oxyn
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
3. Alternatively, select **Actions → livraison → Run workflow** and enter
   that existing version tag, or run:

   ```sh
   gh workflow run livraison.yml --repo so-keyldzn/oxyn -f tag=v0.0.1
   ```

   The manual workflow must first exist on the default branch. The example
   version must be replaced when the configured version changes.
4. Wait for both package jobs, inspect their checks and download/install the
   packages from the draft.
5. Wait for the `manifeste` job. It runs once both package jobs are green,
   with no secret but `GH_TOKEN`: it downloads the draft's `.sig` files,
   requires exactly one `*_aarch64.app.tar.gz` and one `*.AppImage`, each
   with its signature, and uploads `latest.json` with the keys
   `darwin-aarch64` and `linux-x86_64-appimage`. It is the only job that
   uploads `latest.json`, and it refuses a draft that already holds one.
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

The same tests cover `manifeste` and `cle`; they do not prove that an
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
