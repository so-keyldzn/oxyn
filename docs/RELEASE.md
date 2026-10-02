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
the notarized, stapled object inside it is the `.app`.

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
   packages from the draft. Publish the draft manually only after validation.

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
