# ADR-0051 — Oxyn updates itself from the GitHub Releases, in Rust only, and installs on quit

**Status:** proposed · **Date:** 2026-10-02

## Context

A user who downloaded Oxyn has to come back for every new version by hand.
Nothing exists for updates on 2026-10-02: no plugin, no signing key, no
manifest. What the repository already has, and what bounds the choice:

* **The release chain** is `.github/workflows/livraison.yml` and
  `script/livraison`, without `tauri-action`: a pushed `v*` tag builds a
  **draft** GitHub release, which the maintainer publishes by hand
  ([RELEASE](../RELEASE.md)). The repository is public.
* **The packages.** macOS produces an Apple Silicon DMG, signed and notarized
  (#117); Linux produces deb, rpm and AppImage. Intel macOS and Windows are not
  built.
* **The exit.** Oxyn already has an ordered shutdown — transactions, drafts,
  layouts, then the close recorded so that the next launch does not offer
  recovery ([ADR-0038](0038-un-plantage-s-annonce-une-fois.md),
  [ADR-0040](0040-inscrire-la-fermeture-d-une-sortie-forcee.md),
  [ADR-0043](0043-multi-fenetre.md)). A restart that bypasses it loses work
  or shows a false crash screen.
* **The webview is an input surface**: an XSS reaches the Tauri commands
  ([SECURITY](../SECURITY.md#input-surface), item 5). An update is code that
  will run with the user's rights.

Checked on 2026-10-02 in the sources of the versions concerned
([RESEARCH-NOTES](../RESEARCH-NOTES.md#tauri-updater-contracts--checked-on-2026-10-02)):

* `tauri-plugin-updater` **2.13.1** (crates.io, 2026-09-29; a 3.0.0 alpha line
  exists, not taken) checks an endpoint, downloads the archive into memory,
  checks its **minisign** signature against a public key compiled into the
  application, and installs it. It refuses a non-`https` endpoint in a release
  build. Its default comparison only offers a version **strictly greater**
  than the running one. It requires `tauri` 2.12, like
  `tauri-plugin-opener` 2.7.0: the workspace moves from `tauri` 2.11.5 to
  2.12.1 with them.
* The signature covers the archive's bytes, and — since the Tauri CLI 2.12.0 —
  a trusted comment carrying the version the archive was signed for. With
  `requireSignedVersion: true`, the plugin rejects an archive whose signed
  version differs from the one the manifest announces. The manifest itself is
  not signed.
* On macOS the plugin replaces the `.app` by a rename, and asks for an
  administrator password through AppleScript when the rename is refused; on
  Linux it replaces the AppImage pointed to by `$APPIMAGE`, and it would run
  `dpkg`/`rpm` with elevation for a deb or rpm.
* `https://github.com/<owner>/<repo>/releases/latest/download/<asset>` serves
  the asset of the latest **published, non-prerelease** release: a draft is
  invisible there.
* Tauri 2.12.1 offers `restart()`, which never returns and, from another
  thread, sleeps forever once the exit is requested, and `request_restart()`,
  which goes through `RunEvent::ExitRequested` and `RunEvent::Exit` like an
  ordinary exit, then relaunches.

## Decision

### 1. The updater lives in Rust, in `oxyn-desktop`, and the webview never calls it

`tauri-plugin-updater` is a dependency of `oxyn-desktop` alone
([I-08](../../CLAUDE.md#i-08)), with `default-features = false` and **no**
TLS feature: its `rustls-tls` would pull `ring` and install it as the
process's default `CryptoProvider`, under the drivers. It reuses the
`reqwest` that `oxyn-llm` already builds on `rustls` with `aws-lc-rs`;
`cargo tree` shows no `ring` and no new duplicate.
Its JavaScript package `@tauri-apps/plugin-updater` is not installed, and
`capabilities/main.json` grants **no** `updater:*` permission — a test of
`oxyn-desktop` refuses one. The webview reaches the updater only through
narrow Oxyn commands (`get_update_state`, `subscribe_updates`,
`check_for_updates`, `cancel_update`, `download_update`,
`set_automatic_updates`, `restart_to_update`, `open_release_page`,
`take_update_notice`) that never take a URL, a path or a version.

An update is **interface plumbing, not a `Command`**: it touches no database
connection, and no agent can trigger it. The precedent is the menu bar and the
ordered exit ([ADR-0041](0041-registre-d-actions-menus-et-raccourcis.md),
[ADR-0038](0038-un-plantage-s-annonce-une-fois.md)), which do not go through
the bus either ([I-01](../../CLAUDE.md#i-01) is about execution paths to a
driver).

The release page opens in the system browser through `tauri-plugin-opener`'s
free function `open_url`, called from Rust; the opener plugin is **not
registered** — no JavaScript handler, no command, no capability. The URL is a
fixed prefix followed by a version Rust validated as semver. The plugin is taken rather
than spawning `open` or `xdg-open` by hand: its `open` crate already walks the
Linux launchers in order (`xdg-open`, `gio`, `gnome-open`, `kde-open`, WSL),
protects an argument starting with `-` from being read as an option, and
detaches the child — a second implementation would be a second place to audit.

### 2. One endpoint, compiled in: the GitHub Releases of the repository

```
https://github.com/so-keyldzn/oxyn/releases/latest/download/latest.json
```

`latest.json` is the plugin's **static** manifest format: `version`, `notes`,
`pub_date`, and one entry per platform key with `url` and `signature`. Its
keys are `darwin-aarch64` and `linux-x86_64-appimage` — never a bare
`linux-x86_64`, which the plugin would also offer to a deb or rpm install.

**Amendment (2026-10-04).** Linux packages are also built for ARM64, natively
on the `ubuntu-24.04-arm` runner. The manifest gains the key
`linux-aarch64-appimage` — never a bare `linux-aarch64`, for the same reason —
and `manifeste` requires exactly one `*_amd64.AppImage` and one
`*_aarch64.AppImage`, each with its `.sig`, and no other AppImage, where this
ADR says "one AppImage". Nothing else in the decision changes
([RELEASE](../RELEASE.md)).

The endpoint is a constant in a single module (`updates/channel.rs`); it is
neither in `tauri.conf.json` nor in a preference, and the webview cannot
supply one. Only the **stable** channel exists; the isolation is what lets a
beta channel be added later without touching the rest.

`tauri.conf.json` carries the minisign public key and
`requireSignedVersion: true`. The private key exists only in the secrets of
the `release` GitHub environment, which only `v*` tags may deploy to, and in
two offline backups ([RELEASE](../RELEASE.md#updater-signing-key),
[I-03](../../CLAUDE.md#i-03)). The key in use, minisign key ID
`F7DFD985E0B86DFC`, was generated by the maintainer on 2026-10-02. Should
`tauri.conf.json` ever carry the placeholder `REPLACE_WITH_UPDATER_PUBLIC_KEY`
again — a fork, a key being reset — the release workflow refuses it before
building (`script/livraison cle`) and before writing a manifest, and a build
carrying it refuses every update as a signature failure.

### 3. The artifacts are signed after the build, and verified before the manifest

**No build signs.** `createUpdaterArtifacts` stays off: with it, `tauri build`
signs at the end of the bundling, and the key has to be in the environment of
the whole build — every `build.rs` and proc-macro of `Cargo.lock`, Vite and
its plugins, any of which a compromised dependency version could turn into a
reader of that environment. The package jobs build without the key; the macOS
archive is written by `script/apple-release build` from the `.app` **after**
it is signed, notarized and stapled, the way `tauri-bundler` 2.10.1 writes it
(the `.app` at the root of a `.tar.gz`, links kept), and
`script/apple-release verify` checks the `.app` inside it. The AppImage is the
one the bundler wrote: the bundle type it patches into the binary does not
depend on `createUpdaterArtifacts`. A later step, `script/livraison signer`,
alone holds the key: it runs the pinned `tauri signer sign --app-version
<version>`, which writes the same trusted comment as a signing `tauri build`.

**The manifest is written last.** A final job, `manifeste`, runs once both
package jobs are green, with no secret but `GH_TOKEN`: it requires exactly one
macOS archive and one AppImage in the draft, downloads them with their `.sig`,
verifies each pair with `minisign -V` 0.12 (pinned by checksum) against the
public key of `tauri.conf.json`, then checks the signed version, writes
`latest.json` and uploads it. The CLI only warns when the private key does not
match that public key, and a rerun can pair one run's archive with another's
signature: unchecked, either would ship a release every installed copy
refuses as a signature failure. `manifeste` is the only job that uploads
`latest.json`. Since `/releases/latest/download` ignores drafts, nothing is
offered to anyone before the maintainer publishes.

### 4. When it checks, downloads and installs

* **Checks** 60 s after launch, then every 24 h, on `tauri::async_runtime`
  ([I-05](../../CLAUDE.md#i-05)). The due date is computed against the wall
  clock, re-read every hour — a 24 h sleep would stall through the machine's
  sleep. A check is bounded by **30 s**; that bound does not reach the
  download, which the plugin starts with no overall timeout (`Update.timeout`
  is `None`): a slow link must not lose it. Every request of the updater's
  client is HTTPS only, redirects included, and fails after **60 s** without
  receiving a byte — per read, so a stalled stream ends as
  `error{offline, retryable: true}` instead of holding `downloading` for the
  session. `cancel_update` aborts the task, and the HTTP request with it.
* **Downloads** in the background when automatic updates are on. The bytes
  stay **in memory**; the signature is verified by the plugin before anything
  is kept. After a crash or a quit without install, the download simply starts
  again. An archive whose URL is not an asset of the project's releases
  (`https://github.com/so-keyldzn/oxyn/releases/download/…`) is refused before
  any byte is asked for, as `error{server, retryable: false}`: the plugin only
  enforces HTTPS on the endpoint. An archive over **256 MiB**, announced or
  received, is dropped as `error{server, retryable: true}`: the plugin buffers
  the whole body before checking its signature, so an unverified asset must
  not choose that size — the largest published, the v0.0.1 AppImage, weighs
  89 MB.
* **Installs on quit**, never by forcing a restart. A "Restart now" action is
  offered; it runs the ordered exit, then installs, then calls
  `request_restart()` — never `restart()`. The close is recorded before the
  install, so the relaunch shows no recovery screen. `cancel_exit` clears the
  restart intention, otherwise a later ⌘Q would relaunch.
* **"Restart now" names the work it would stop.** `restart_to_update`
  answers `busy{running, exports}` while statements run or exports are being
  written in any window; exports are counted by a guard around the export.
  Confirmed, the exit does not wait for an export: its destination keeps its
  previous content, and a stray `.oxyn-export-*.part` may remain beside it.
* **The Dock's Quit and logout** (`RunEvent::Exit`) also install, on macOS
  only, after the windows are gone, on a helper thread the main thread waits
  for at most **10 s** — measured on 2026-10-02, extracting the 12 MB archive
  of a 25 MB bundle takes 0.6 s. Past that bound the process ends without
  waiting further; 10 s is a bound for a slow disk, not an expected duration.
  Accepted risk: the plugin extracts the whole archive before touching the
  installed `.app`, then swaps it by two renames microseconds apart; cut
  between them, `/Applications` lacks Oxyn.app until the backup left in
  `$TMPDIR` is moved back. On Linux the plugin writes the new AppImage over
  seconds, so cutting it would leave a truncated image: such an exit installs
  nothing, and the next launch downloads the update again.
* **Where Oxyn cannot write** (the `.app` in a folder the user does not own,
  on a mounted image under `/Volumes`, or translocated by Gatekeeper),
  nothing is installed on quit: the administrator prompt only ever appears
  after a click on "Restart now". The folder is probed on the blocking pool
  once a download is verified, never at launch: the launch reads only
  `updates.json` and `update-notice.json`, each bounded at 64 KiB.
* **One operation at a time.** A check during `checking`, `downloading` or
  `ready` does nothing; `ready` lasts until the restart, or until automatic
  updates are turned off.
* **Failures.** A network or server failure in the background is kept as
  `error{offline|server, retryable: true}`, shown in Settings and nowhere
  else, and retried at the next due date; a signature or install failure is
  always shown.
* **Automatic off** rests in `disabled{user}`; a manual check still runs
  from there. Turning it off, as the switch says (UX-SPEC), aborts a
  download under way and drops a `ready` update with its bytes, under a new
  ticket so a late completion is refused: quitting then installs nothing. A
  download started by hand while it was already off stays the user's.
  Concurrent saves of the preference are serialized from the write to the
  publication, each through a temporary file of its own.
* **Release notes** are capped at 4 KiB, cut on a character boundary
  ([I-09](../../CLAUDE.md#i-09)), and rendered as plain text.

### 5. A preference for the application, not the workspace

`app_config_dir()/updates.json`, `{"format":1,"automatic":true}`, readable
without Oxyn ([I-11](../../CLAUDE.md#i-11)). An unknown field is ignored; a
corrupt file counts as the default.
`OXYN_UPDATES=off` in the environment locks updates off, for an administered
or offline machine.

What the exit tells the next launch crosses the restart in a second open file
beside it, `app_config_dir()/update-notice.json`, in one of two shapes:

```json
{"format":1,"type":"installed","from":"0.0.2","to":"0.0.3"}
{"format":1,"type":"installFailed","version":"0.0.3","message":"…"}
```

`message` is the sentence shown to the user, followed by the plugin's error;
it carries no secret. The file is written atomically by the install, read and **removed** at the next
launch, before the notice is shown — a notice is said once. `installed` is
shown only if `to` is the version that launches (an older copy started from
elsewhere says nothing); a file that does not parse is ignored. Both files
are bounded at 64 KiB when read. A future `format` is a new value of that
field, never a silent change of the shapes above.

### 6. Where it applies

| Installation | Behavior |
|---|---|
| macOS `.app` (DMG) | updates itself |
| Linux AppImage (`$APPIMAGE` set) | updates itself |
| Linux deb, rpm | disabled: "managed by your package manager" — the plugin would otherwise run `dpkg`/`rpm` with elevation behind the package manager's back |
| Development build | disabled |
| Windows, Intel macOS | not built today; no manifest key |

## Consequences

* **+** An installed Oxyn stays current without the user coming back, and
  without ever losing work to a restart.
* **+** The webview gains no power: no plugin permission, no URL, no path. An
  XSS can at most ask for a check or a restart the user would be offered
  anyway.
* **+** A compromised GitHub account or CDN cannot ship code: the archive must
  carry a minisign signature from a key GitHub never holds.
* **+** The version-lie replay is closed: `requireSignedVersion` binds each
  archive to the version it was signed for, so a forged `latest.json` cannot
  offer an old, genuinely signed archive under a higher version number.
* **−** **`latest.json` is not signed.** Whoever controls the endpoint can
  still **withhold** updates — serve an old manifest, or none — and keep users
  on a vulnerable version — and choose the release notes Settings shows
  beside a genuine archive. Accepted: the alternative is a second signature
  chain for the manifest, and the user sees the running version in Settings;
  the notes are capped at 4 KiB and rendered as plain text, so a forged note
  is social engineering, not code.
* **−** **The signing step still trusts the Tauri CLI.** The key reaches one
  process, the `@tauri-apps/cli` binary pinned in the lockfile; the build's
  hundreds of crates and npm packages never see it.
* **−** **GitHub availability is Oxyn's.** If GitHub is down, checks fail
  silently until it returns.
* **−** **v0.0.1 users update by hand once.** It ships without the updater;
  only versions built after this ADR can update themselves.
* **−** **Losing the private key ends updates.** Users then have to download a
  build signed with a new key by hand; rotation is only possible while the old
  key exists ([RELEASE](../RELEASE.md#updater-signing-key)).
* **−** deb and rpm users do not get updates from Oxyn; they wait for their
  package manager, which today has no repository to read from.

**Exit cost:** low on the code side — the updater is one module tree in
`oxyn-desktop` (`updates/`) plus its commands, and nothing outside the desktop
crate knows it exists. High on the trust side: the public key is compiled
into every installed copy. Changing the endpoint or the key requires one
release, signed with the old key, that carries the new ones.

**Reconsider if** a Windows or Intel macOS build is added (new manifest keys,
the Windows installer exits the app itself); if GitHub Releases limits or
changes `/releases/latest/download`; if a beta channel is needed; if the
`tauri-plugin-updater` 3.x line becomes stable, or 2.x stops receiving fixes.

## Rejected alternatives

| Alternative | Reason for rejection |
|---|---|
| The plugin's JavaScript API (`@tauri-apps/plugin-updater`) called from the webview | Requires `updater:*` permissions: an XSS could then trigger checks, downloads and installs, and versions before 2.12.0 even let the webview relax the version comparison. Rust keeps the decision where the webview cannot reach it |
| `tauri-action` to build and publish the release | The release chain already exists without it, with its own draft and refusal rules ([RELEASE](../RELEASE.md)); a second publisher would race the first. The manifest is a few lines of `script/livraison`, tested like the rest |
| Persisting the downloaded archive to install it at a later launch | A file on disk is a file someone else can replace between the check and the install; the signature would have to be re-verified, and a crash mid-write leaves a half archive. Downloading again costs one transfer |
| A per-workspace preference | An update replaces the application, not a workspace: a workspace that says "off" would not stop another that says "on" on the same machine |
| A dedicated update service (CrabNebula Cloud, a CDN of our own) | A second provider to trust and pay, for what the GitHub Releases of a public repository already serve; reopens with a Windows build or a beta channel if GitHub becomes the limit |
| `restart()` for "Restart now" | It never returns and, from a thread other than the main one, sleeps forever: the ordered exit would never finish its steps |
| Leaving `requireSignedVersion` off | Nothing to stay compatible with — no release carries the updater yet — and off, the unsigned manifest can replay an old signed archive under a higher version |
