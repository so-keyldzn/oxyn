# Verification notes

> **Authority**: the external versions and facts cited elsewhere in the
> repository. Every value below carries its source and its date. An undated
> value is a stale value nobody has spotted yet.

Related invariant: [I-12](../CLAUDE.md#i-12) — no version or external limit
copied from memory.

## How to re-check

```bash
.claude/hooks/verifier_versions.py
```

The script queries crates.io and the Rust stable channel, compares them with
the values of this file and reports the gaps. It changes nothing: deciding on
a version bump is a human's call. The `/versions` command does the same while
explaining the gaps.

## Rust toolchain

| Fact | Value | Source | Checked on |
|---|---|---|---|
| Current Rust stable | `1.98.1` (48a229cea, 2026-09-01) | `https://static.rust-lang.org/dist/channel-rust-stable.toml` | 2026-09-05 |
| Toolchain pinned by the repository | `1.98.1` | `rust-toolchain.toml` | 2026-09-05 |
| Edition required by `gpui` | `2024` | crates.io API, `gpui@0.2.2` | 2026-09-05 |
| Minimum Rust for edition 2024 | `1.85` | Edition 2024 stabilized in Rust 1.85 | 2026-09-05 |

### MSRVs imposed by dependencies

Read in `~/.cargo/registry/src/.../<crate>/Cargo.toml`, field
`rust-version`. This is the real floor of the workspace: `cargo` refuses to
build a package whose `rust-version` exceeds the toolchain.

| Crate | `rust-version` | Checked on |
|---|---|---|
| `wasmtime` `48.0.1` | **`1.95.0`** | 2026-09-05 |
| `sqlx` `0.9.0`, `sqlx-core`, `sqlx-postgres` | `1.94.0` | 2026-09-05 |
| `arrow` `59.3.0` | `1.85` | 2026-09-05 |
| `rusqlite` `0.37.0` | none | 2026-09-05 |
| `gpui` `0.2.2` | none | 2026-09-05 |

> **The floor is `1.95.0`, imposed by `wasmtime`.** It does not show in the
> default build: `wasmtime` sits behind the `wasm-host` feature of
> `oxyn-plugin`, which is disabled. Only `sqlx` (`1.94.0`) makes
> `cargo check` fail today. The day someone enables `wasm-host`, `1.95.0` is
> what is needed — hence the workspace `rust-version` set to `1.95`, and not
> to `1.94` as the only observed error would suggest.

> **Gap resolved on 2026-09-05.** The development machine was on `1.89.0`,
> nine minor versions behind stable, and `rust-toolchain.toml` pinned that
> value — the one ADR-0008 explicitly rules out. `cargo check` settled it:
> `sqlx 0.9.0` requires `1.94.0`. The toolchain moved to `1.98.1`, following
> the recommendation of
> [ADR-0008](adr/0008-chaine-outils-rust.md).

## Tauri interface and front end

Read on 2026-09-15 from the npm registry (`npm view <package> version`) and
on crates.io (API `/api/v1/crates/<crate>`), for [ADR-0029](adr/0029-interface-tauri-shadcn.md).
Versions are written **exact** in `apps/desktop/package.json` and the root
`Cargo.toml`; `pnpm-lock.yaml` freezes the rest of the graph.

### Crates

| Crate | Version | `rust-version` | Checked on |
|---|---|---|---|
| `tauri` | `2.12.1` | `1.90` | 2026-10-02 |
| `tauri-build` | `2.7.1` | `1.90` | 2026-10-02 |
| `tauri-plugin-dialog` | `2.7.3` | `1.77.2` | 2026-09-15 |
| `tauri-plugin-updater` | `2.13.1` | `1.90` | 2026-10-02 |
| `tauri-plugin-opener` | `2.7.0` | `1.90` | 2026-10-02 |
| `rfd` | `0.16.0` | — (not declared) | 2026-09-24 |

`rfd` shows the startup failure dialog, before the Tauri application
exists. The version is the one `tauri-plugin-dialog@2.7.3` already resolves
(`Cargo.lock`, requirement `0.16`): the latest published is `0.17.2`
(crates.io, 2026-09-24), but taking it would put two `rfd` in the graph.
Declared without default features: the plugin's (`gtk3`,
`common-controls-v6`) unify on the same crate.

`tauri-plugin-updater` and `tauri-plugin-opener` serve the automatic updates
([ADR-0051](adr/0051-automatic-updates-from-github-releases.md)); both
published on 2026-09-29, both require `tauri` 2.12, which moved `tauri` from
2.11.5 to 2.12.1 and `tauri-build` from 2.6.3 to 2.7.1 (both published on
2026-09-30), and with them `tao` 0.35.3 → 0.37.1, `wry` 0.55.1 → 0.57.0,
`muda` 0.19.3 → 0.20.0 in `Cargo.lock`. A 3.0.0 alpha line exists for `tauri`
and both plugins: not taken. The updater is declared with
`default-features = false` and no TLS feature, hence without `rustls-tls`,
`system-proxy` and `zip`: its `rustls-tls` enables `reqwest/rustls-no-provider`
plus its own `rustls` dependency, which would pull `ring` and install it as
the process's default `CryptoProvider`. It reuses the `reqwest` 0.13.4 that
`oxyn-llm` builds on `rustls` with `aws-lc-rs`; on 2026-10-02,
`cargo tree -p oxyn-desktop -i ring` prints nothing, and the graph keeps one
`reqwest` and one `rustls` (0.23.45). The opener plugin is not registered:
Rust calls its free function `open_url`, which needs no `AppHandle`. Its npm
package `@tauri-apps/plugin-updater` (2.13.1) and the `tauri-plugin-process`
crate (2.4.0) are **deliberately not taken**: the webview never drives the
update, and Rust restarts through `AppHandle::request_restart`. Sources:
crates.io API `/api/v1/crates/<crate>/<version>`, npm registry, read on
2026-10-02.

Re-read after that bump, on 2026-10-02, the two facts below marked "to
re-check at every bump of `tauri`" still hold: `tauri` 2.12.1
`src/path/desktop.rs` computes `app_log_dir` as before, and `tao` 0.37.1
still registers `applicationWillTerminate:` only, and stops through
`[NSApp stop:]` (`src/platform_impl/macos/app_delegate.rs`, `app_state.rs`).

Behaviors of the message dialog that
[ADR-0037](adr/0037-dialogue-natif-pour-les-confirmations-critiques.md) relies
on, checked in the source on 2026-09-25 — **to re-read at every bump of the
plugin or of `rfd`**:

- `tauri-plugin-dialog` 2.7.3, `src/lib.rs`, `MessageDialogBuilder::show`: the
  callback receives `true` for `Ok`, `Yes`, or a custom button whose label
  **equals** the confirmation one; `false` for any other outcome, closing
  included. Two equal labels would make Cancel a confirmation;
- same crate, `src/desktop.rs`, `show_message_dialog`: the dialog opens
  through `run_on_main_thread`, whose failure is ignored; the callback is then
  never called, and a dropped channel must count as a refusal;
- `rfd` 0.16.0 (dependency of the plugin), `src/backend/macos/message_dialog.rs`:
  buttons are added to the `NSAlert` in order, confirmation first; the first
  button of an `NSAlert` answers Enter, and the plugin offers no way to
  designate another one.

### npm packages

| Package | Chosen version | Latest published | Why the gap |
|---|---|---|---|
| `pnpm` (`packageManager`) | `11.1.2` | `12.4.2` | version installed on the dev machine; bumping is a deliberate commit |
| `@tauri-apps/cli` · `@tauri-apps/api` | `2.12.1` · `2.12.1` | same | read on 2026-10-02. The CLI moved from 2.11.4 for the updater: from 2.12.0 it signs the version into the updater signature, which `requireSignedVersion` checks ([updater contracts](#tauri-updater-contracts--checked-on-2026-10-02)). `@tauri-apps/api` moved from 2.11.1 with the `tauri` crate: `tauri build` refuses a different major or minor |
| `@tanstack/react-start` · `react-router` · `router-plugin` | `1.168.54` · `1.170.36` · `1.168.38` | same | — |
| `@tanstack/react-query` · `react-virtual` · `react-form` | `5.102.8` · `3.14.13` · `1.33.5` | same | — |
| `@tanstack/react-store` · `react-pacer` | `0.11.1` · `0.23.0` | same | `react-hotkeys` (`0.10.0`, which declared itself **alpha** in its README) was removed on 2026-09-25: the dispatcher of the action registry replaces it ([ADR-0041](adr/0041-registre-d-actions-menus-et-raccourcis.md), point 3) |
| `@tanstack/react-table` | `8.21.3` | `9.2.4` | shadcn components and examples assume the v8 API |
| `shadcn` (CLI) · `@base-ui/react` | `4.21.0` · `1.8.0` | same | `base-nova` style |
| `@hugeicons/react` · `@hugeicons/core-free-icons` | `1.1.10` · `4.3.3` | same | UX-SPEC mandates Hugeicons |
| `react` · `react-dom` | `19.3.0` | same | — |
| `vite` · `@vitejs/plugin-react` | `8.3.0` · `6.1.1` | same | — |
| `tailwindcss` · `@tailwindcss/vite` | `4.3.3` | same | — |
| `typescript` | `6.0.3` | `7.0.2` | see ADR-0029, rejected alternatives |
| `storybook` · `@storybook/react-vite` · addons | `10.6.0` | same | — |
| `vitest` · `@vitest/browser-playwright` | `4.1.11` | `5.0.1` | `@storybook/addon-vitest@10.6.0` requires `vitest ^3 \|\| ^4` |
| `playwright` | `1.63.0` | same | Chromium only, for the stories |
| `@uiw/react-codemirror` · `@codemirror/lang-sql` | `4.25.11` · `6.10.0` | same | — |
| `zod` | `4.6.5` | same | read on 2026-09-16; validates IPC responses ([ADR-0031](adr/0031-validation-des-reponses-ipc.md)). Had been declared since ADR-0029 and imported nowhere |
| `lexical` · `@lexical/react` | `0.51.0` | same | read on 2026-09-24 (`npm view`, dist-tag `latest`, published on 2026-09-23); the assistant's question field and its `@` mentions. `@lexical/react` declares `yjs` as a peer: it only serves the collaboration plugin, not imported |

| `@xyflow/react` | `12.11.6` | same | read on 2026-09-24 (published on 2026-09-01); the table diagram. Peers `react >=17` |
| `@dagrejs/dagre` | `3.1.1` | same | read on 2026-09-24 (published on 2026-08-08); layout of the diagram. **Not** `dagre` 0.8, abandoned, nor `elkjs` |
| `shiki` | `4.4.3` | same | read on 2026-09-24 (published on 2026-08-10); highlighting of code in responses, with the **JavaScript engine** (`shiki/engine/javascript`) and tokens, never `codeToHtml` |
| `mermaid` | `11.17.2` | `12.0.0` | read on 2026-09-24 (11.17.2 published on 2026-08-25, 12.0.0 on 2026-09-10). **12 targets Safari 17.4+ and ES2024**, whereas the target is macOS 13.0 (`minimumSystemVersion`), shipped with Safari 16: we stay on the latest 11.x. To reopen when the minimum target guarantees a WebKit 17.4 |

### Facts that decided the code

| Fact | Source | Checked on |
|---|---|---|
| Tauri only serves static content: SSG, SPA or MPA, no SSR | `https://v2.tauri.app/start/frontend/` | 2026-09-15 |
| Start's SPA mode writes a shell (`/_shell.html` by default, `prerender.outputPath`) | `https://tanstack.com/start/latest/docs/framework/react/guide/spa-mode` | 2026-09-15 |
| Build targets from Tauri's Vite guide: `chrome105` on Windows, `safari13` elsewhere | `https://v2.tauri.app/start/frontend/vite/` | 2026-09-15 |
| Tauri injects nonces and hashes into the CSP **in development too**; a nonce cancels `'unsafe-inline'`, and Vite's inline scripts are blocked: the window stays blank with no visible error | `https://v2.tauri.app/reference/config/` and observed on macOS 26.2 | 2026-09-15 |
| `@storybook/tanstack-react@10.6.0` bundles `@tanstack/router-core@1.171.30`; with `react-router@1.170.36`, every story fails on `path.endsWith is not a function` | observed, `vitest --project storybook` | 2026-09-15 |
| A command without `async` runs **on the main thread**, unless declared `#[tauri::command(async)]`; an `async` command can only take `State<'_, T>` by returning a `Result` | `https://v2.tauri.app/develop/calling-rust/` | 2026-09-15 |
| `@xyflow/react` sets `pointer-events: none` on a node that is neither selectable nor draggable: a button in the node no longer receives the click without `pointer-events-auto` on its content | observed, `vitest --project storybook`, 12.11.6 | 2026-09-24 |
| The React Flow attribution (a link to reactflow.dev) can only be removed, according to its authors, with a React Flow Pro subscription: Oxyn keeps it | `https://reactflow.dev/learn/troubleshooting/remove-attribution` | 2026-09-24 |
| shiki's default engine is Oniguruma **compiled to WebAssembly**; the production CSP (`script-src 'self'`, without `'wasm-unsafe-eval'`) refuses it. The JavaScript engine transpiles patterns into native `RegExp`s; with `target: 'auto'` (default) it only uses the `v` flag (ES2024) if the engine has it, otherwise the `u` flag | `https://shiki.style/guide/regex-engines` | 2026-09-24 |
| mermaid 12: "built to target Safari 17.4+ and ES2024"; ELK becomes the default layout | `https://github.com/mermaid-js/mermaid/releases/tag/mermaid%4012.0.0` | 2026-09-24 |
| mermaid 11.17.2 refuses that a directive or a diagram's header change a key listed in `secure` (by default `secure`, `securityLevel`, `startOnLoad`, `maxTextSize`, `suppressErrorRendering`, `maxEdges`), and sanitizes every directive (`sanitizeDirective`) | sources of `mermaid@11.17.2`, `dist/chunks/mermaid.core/chunk-DU6HZSFF.mjs` | 2026-09-24 |
| Under the CSP of `tauri.conf.json` served as a header, a production build (target `safari13`) highlights code, draws mermaid as a `data:` URL, the table diagram and the chart **without any violation**, in Chromium and WebKit 26.6 (Playwright 1.63.0). Not reproduced: the hashes Tauri adds to the CSP itself, and the WebKit of macOS 13 | observed, `vite build` + Playwright harness | 2026-09-24 |
| `PathResolver::app_log_dir`: `home_dir/Library/Logs/<identifier>` on macOS, `data_local_dir/<identifier>/logs` elsewhere, through the `dirs` crate. `logging::directory` recomputes it through `directories` (same `dirs-sys`) because the backend opens before the application exists: to re-check at every bump of `tauri` | sources of `tauri@2.11.5`, `src/path/desktop.rs` | 2026-09-24 |
| On macOS, tao handles `applicationWillTerminate` (→ `Event::LoopDestroyed` → `RunEvent::Exit`) but not `applicationShouldTerminate`: the predefined Quit of the menu, the Dock's, and logging out terminate the application without `ExitRequested`, which nothing can hold back ([ADR-0038](adr/0038-un-plantage-s-annonce-une-fois.md)). `tao@0.37.0`, the latest published, does not register `applicationShouldTerminate:` either, and `tauri@2.11.5` does not add it; tao's exit (`app.exit`) goes through `[NSApp stop:]`, not `terminate:` ([ADR-0040](adr/0040-inscrire-la-fermeture-d-une-sortie-forcee.md)) | sources of `tao@0.35.3` (`src/platform_impl/macos/app_delegate.rs`, `app_state.rs`), of `tao@0.37.0` (same file, tag `tao-v0.37.0`), of `tauri@2.11.5` and of `tauri-runtime-wry@2.11.4` (`src/lib.rs`) | 2026-09-25 |
| SQLite automatically rolls back the open transaction of a connection being closed; after some errors (including `SQLITE_BUSY` and `SQLITE_INTERRUPT`), a transaction may be rolled back automatically, and only `sqlite3_get_autocommit` reveals it ([ADR-0039](adr/0039-etat-de-transaction-d-une-session.md)) | `https://www.sqlite.org/c3ref/close.html`, `https://www.sqlite.org/c3ref/get_autocommit.html` | 2026-09-25 |
| `rusqlite@0.37.0` exposes `Connection::is_autocommit`; `sqlx-postgres@0.9.0` keeps the `ReadyForQuery` state (`Idle`, `Transaction`, `Error`) private — `PgConnection::in_transaction` is `pub(crate)` and confuses `Error` with `Idle`; `sqlx-core@0.9.0` `Connection::is_in_transaction` only counts transactions opened by sqlx (`transaction_depth`), not a typed `BEGIN` ([ADR-0039](adr/0039-etat-de-transaction-d-une-session.md)) | installed sources: `rusqlite-0.37.0/src/lib.rs`, `sqlx-postgres-0.9.0/src/connection/mod.rs` and `src/message/ready_for_query.rs`, `sqlx-core-0.9.0/src/connection.rs` | 2026-09-25 |
| `sqlx` logs the whole text of a query on the `sqlx::query` target: at `debug` by default, at `warn` beyond one second | sources of `sqlx-core@0.9.0`, `src/connection.rs` and `src/logger.rs` | 2026-09-24 |
| Vitest's browser server starts from port 63315 and inherits the `server` of the Vite configuration: with `server.strictPort: true`, two worktrees running the stories at the same time fail on "Port 63315 is already in use". `browser.api: { strictPort: false }` restores Vite's fallback to the next port; `port: 0` is useless, Vitest replaces a zero port with 63315 | sources of `@vitest/browser@4.1.11` (`dist/index.js`, plugin `vitest:browser:config`) and of `vitest@4.1.11` (`resolveApiServerConfig`), and observed, two simultaneous `make front-tests` | 2026-09-24 |
| `esbuild` and `unrs-resolver` ship their binary as an optional dependency: their install scripts are refused (`allowBuilds`) | `pnpm install`, pnpm 11.1.2 | 2026-09-15 |
| `LexicalTypeaheadMenuPlugin` sets `role="listbox"` and `aria-label="Typeahead menu"` on its anchor **at every attachment** — the anchor is removed then put back at every keystroke —, and leaves `aria-activedescendant` on `typeahead-item-0` when the list empties: axe fails with `aria-valid-attr-value` and, on a list with no option, with `aria-required-children` | sources of `@lexical/react@0.51.0` (`shared/LexicalMenu.tsx`) and observed, `vitest --project storybook` | 2026-09-24 |

### Upstream follow-ups

What Oxyn expects from a library rather than working around it. Each row
carries the decision that makes us wait; it is re-checked with `/versions`.

| Expected | Upstream state | Source | Checked on |
|---|---|---|---|
| `sqlx` exposes PostgreSQL notices (`NoticeResponse`) to the caller, per connection — which unblocks the `Messages` tab. Decision of 2026-09-24 (audit, D13): we wait, the driver does not move to `tokio-postgres` | **Nothing shipped.** Latest version `0.9.0` (crates.io), the one in `Cargo.lock`. On `main` (`b54008a`, 2026-09-14), `sqlx-postgres/src/connection/stream.rs` still decodes the notice to log it on `sqlx::postgres::notice` and discard it. `PgSeverity` is re-exported, `Notice` is not. Issue [#3621](https://github.com/transact-rs/sqlx/issues/3621) "Expose a stream of `NoticeResponse`s from Postgres", opened on 2024-12-01, has no comment and no linked PR. The repository moved from `launchbadge/sqlx` to `transact-rs/sqlx` after 0.9.0 | crates.io `/api/v1/crates/sqlx`; GitHub API (issue, search `NoticeResponse`, content of `main`); `CHANGELOG.md` of `main`, no entry after 0.9.0 | 2026-09-25 |

## CI and GitHub delivery

Read on 2026-09-24 from the GitHub API (`/repos/<repo>/releases/latest`, then
`/repos/<repo>/commits/<tag>` for the SHA). Workflows pin the SHA, not the
tag: a tag moves, a SHA does not.

| Fact | Value | Source | Checked on |
|---|---|---|---|
| `actions/checkout` | `v7.0.1`, `3d3c42e5aac5ba805825da76410c181273ba90b1`, published on 2026-07-20 | GitHub API | 2026-09-24 |
| `actions/setup-node` | `v7.0.0`, `820762786026740c76f36085b0efc47a31fe5020`, published on 2026-07-14 | GitHub API | 2026-09-24 |
| `pnpm/action-setup` | `v6.1.0`, `ea17c68df8912ef543352723c149a84f56e3d413`, published on 2026-09-05; reads `packageManager` through `package_json_file` when `version` is absent | GitHub API, `action.yml` at that SHA | 2026-09-24 |
| `Swatinem/rust-cache` | `v2.9.2`, `6323deb102c322ba6fcbdcafc7e3dddab59af2b6`, published on 2026-08-06 | GitHub API | 2026-09-24 |
| `Swatinem/rust-cache` only saves the cache after a successful job, unless `cache-on-failure: true` (`post-if: "success() \|\| env.CACHE_ON_FAILURE == 'true'"`) | `action.yml` and `src/restore.ts` at that SHA | 2026-09-24 |
| `actions/cache` | `v6.1.0`, `55cc8345863c7cc4c66a329aec7e433d2d1c52a9`, published on 2026-06-26; only saves after a successful job (`post-if: "success()"`) | GitHub API, `action.yml` at that SHA | 2026-09-24 |
| Node required by the front end | vite 8.3.0: `^20.19.0 \|\| >=22.12.0`; vitest 4.1.11: `^20.0.0 \|\| ^22.0.0 \|\| >=24.0.0`; CI takes the 22 line, the dev machine's (22.23.2) | `engines` field of the installed packages | 2026-09-24 |
| Tauri system libraries on Debian/Ubuntu | `libwebkit2gtk-4.1-dev build-essential curl wget file libxdo-dev libssl-dev libayatana-appindicator3-dev librsvg2-dev` | `tauri-apps/tauri-docs`, branch `v2`, `src/content/docs/start/prerequisites.mdx`, commit `2e513e3` of 2026-08-20 | 2026-09-24 |
| `keyring` 4.2.0 on Linux goes through `zbus-secret-service-keyring-store` and `secret-service` 5.2.0: pure Rust, without `libdbus` | `cargo tree --target x86_64-unknown-linux-gnu` | 2026-09-24 |
| `cargo-nextest` CI binary | `0.9.146`; `universal-apple-darwin` SHA-256 `39785160b3c2f6ed9a765049cf4fa79f3b39aa02eb7598a5a0e2a1a0b9ffb9a8`; `x86_64-unknown-linux-gnu` SHA-256 `682c21b777c333e96fd532e114d3a5a894e0729ab88d94c0a9f20f8419695428` | [nextest release cargo-nextest-0.9.146](https://github.com/nextest-rs/nextest/releases/tag/cargo-nextest-0.9.146) and its `.sha256` assets | 2026-10-03 |
| `cargo-deny` CI binary | `0.20.2`; `aarch64-apple-darwin` SHA-256 `fe67d82a10d8597a3549364cb733a3f9cc1bfff9031b7ae46384a9f2a72090c3`; `x86_64-unknown-linux-musl` SHA-256 `9f12ed4c49936e09b48bf862b595cde2fe64fcbd9d74dfacac6131ca824c8d5f` | [cargo-deny release 0.20.2](https://github.com/EmbarkStudios/cargo-deny/releases/tag/0.20.2) and its `.sha256` assets | 2026-10-03 |
| CI failed at every push since at least 2026-09-21, in about twenty seconds: pnpm missing from the runner, `make qualite` stopped before the front end | log of run `36034960589` | 2026-09-24 |
| None of the runs of the gate had saved a Cargo cache: all of them failed, and the step "Post Restaurer le cache Cargo" was `skipped`. Only the pnpm caches existed. On Linux, the stories took 275 s (1045 tests) before the failure | runs `36037137682` and `36036585873`, `GET /actions/caches` | 2026-09-24 |
| Branch protection is refused on this repository: "Upgrade to GitHub Pro or make this repository public" (403). Nothing therefore prevents merging a PR whose CI fails | `GET /repos/so-keyldzn/oxyn/branches/main/protection` | 2026-09-24 |
| Still refused, branch protection as well as rulesets (403, same message): the aggregate `qualite` job of ADR-0045 is not yet required by anything | `GET /repos/so-keyldzn/oxyn/branches/main/protection`, `GET /repos/so-keyldzn/oxyn/rulesets` | 2026-09-25 |
| The public repository's active ruleset `24429281` (`main requires qualite`) now requires pull requests (zero approvals) and the `qualite` check from GitHub Actions app `15368` on `main`; it forbids deletion and non-fast-forward updates. Administrator bypass is limited to PR merges (`pull_request`), so direct pushes are refused for everyone | `GET /repos/so-keyldzn/oxyn/rulesets/24429281`, `GET /repos/so-keyldzn/oxyn/rules/branches/main` | 2026-10-03 |
| On the `pull_request` event, `actions/checkout` checks out the merge commit by default (`GITHUB_SHA`, `refs/pull/<n>/merge`), whose first parent is the tip of the base: `git diff HEAD^1 HEAD` with `fetch-depth: 2` gives the files of the PR (`script/zones-ci`) | `https://docs.github.com/en/actions/reference/workflows-and-actions/events-that-trigger-workflows`, § `pull_request`: `GITHUB_SHA` "Last merge commit on the `GITHUB_REF` branch"; no run if the PR has conflicts | 2026-09-25 |
| `vitest run --changed [since]` ("Run tests that are affected by the changed files") and `--passWithNoTests` exist in vitest 4.1.11; `eslint --no-warn-ignored` in eslint 9.39.5; `prettier --ignore-unknown` in prettier 3.9.6 — options of `script/verif-rapide` | `--help` of the installed binaries | 2026-09-25 |
| No job started any more on PRs #31, #32 and #33: "The job was not started because recent account payments have failed or your spending limit needs to be increased" (0 step, no runner). The macOS + Linux matrix on every PR, with 4 macOS jobs per PR, had exhausted the quota. Hence Linux only on PRs (`qualite.yml`) | annotations of the jobs of run `36046972599` | 2026-09-24 |

## Apple release contracts — checked on 2026-10-01

| Fact | Consequence | Source |
|---|---|---|
| Developer ID Application signs apps distributed outside the Mac App Store; notarization accepts a team API key, issuer ID and private-key path | Delivery requires this identity and all seven documented secrets; no unsigned fallback | [Tauri macOS signing](https://v2.tauri.app/distribute/sign/macos/), [Apple Developer ID](https://developer.apple.com/help/account/certificates/create-developer-id-certificates/) |
| The pinned CLI `2.11.4` resolves to Tauri commit `8909f221d1515955fc843808032bdc5d62209c96`; `APPLE_CERTIFICATE` and `APPLE_CERTIFICATE_PASSWORD` import a temporary keychain; normal `Drop` deletes it; the app is notarized/stapled and the DMG signed | Use Tauri's credential handling on disposable hosted runners; verify the app's ticket and both signatures after bundling | [sign.rs](https://github.com/tauri-apps/tauri/blob/8909f221d1515955fc843808032bdc5d62209c96/crates/tauri-bundler/src/bundle/macos/sign.rs), [keychain.rs](https://github.com/tauri-apps/tauri/blob/8909f221d1515955fc843808032bdc5d62209c96/crates/tauri-macos-sign/src/keychain.rs), [dmg/mod.rs](https://github.com/tauri-apps/tauri/blob/8909f221d1515955fc843808032bdc5d62209c96/crates/tauri-bundler/src/bundle/macos/dmg/mod.rs) |
| An unset GitHub secret evaluates to an empty string; `gh secret set` accepts stdin; manual dispatch requires the workflow on the default branch | Check credentials explicitly, document direct secret transfer and require an existing version tag for manual builds | [GitHub secrets](https://docs.github.com/en/actions/how-tos/write-workflows/choose-what-workflows-do/use-secrets), [manual dispatch](https://docs.github.com/en/actions/how-tos/manage-workflow-runs/manually-run-a-workflow) |
| `cargo-about` 0.9.2's two archive checksums still match the values in the Licenses section below | Reuse the quality workflow's version and verified checksums in delivery; `--exiger` otherwise stops the release build | [release 0.9.2](https://github.com/EmbarkStudios/cargo-about/releases/tag/0.9.2), macOS ARM and Linux musl `.sha256` assets fetched on this date |

Team API key scope was rechecked on 2026-10-02: keys apply across the account’s
apps. The dedicated notarization key uses Developer access, as documented by
Tauri; its private key is downloadable once.
Source: [Apple API key help](https://developer.apple.com/help/app-store-connect/get-started/app-store-connect-api/).

## Wasmtime security patch — checked on 2026-10-02

The workspace now requires Wasmtime **48.0.5**, with its matching Cranelift
0.135.5 family in `Cargo.lock`. The crates.io sparse index lists 48.0.5 as
not yanked, with Rust 1.95.0 still its minimum; 49.0.2 requires Rust 1.96.0,
above the workspace's `rust-version` floor of 1.95 (the pinned 1.98.1
toolchain would build it, but the declared minimum would become false).

- 48.0.3 fixed the fuel accounting advisories RUSTSEC-2026-0315 and
  RUSTSEC-2026-0316 that blocked `make qualite` on 48.0.1.
- 48.0.4 fixes RUSTSEC-2026-0325 (GHSA-cfhf-m2cr-62wj), RUSTSEC-2026-0326
  (GHSA-hw8m-q44c-ggrf) and RUSTSEC-2026-0327 (GHSA-32h6-97mm-8q3c, rated
  critical), all three published on 2026-10-02 with
  `>= 48.0.4, < 49.0.0` or `>= 49.0.2` as patched ranges. 48.0.5 only
  republishes the release artifacts 48.0.4 failed to ship, so it is the
  version taken.

No advisory is ignored to permit delivery.
Sources: [registry index](https://index.crates.io/wa/sm/wasmtime),
[RUSTSEC-2026-0327](https://rustsec.org/advisories/RUSTSEC-2026-0327),
[GHSA-32h6-97mm-8q3c](https://github.com/bytecodealliance/wasmtime/security/advisories/GHSA-32h6-97mm-8q3c),
[48.0.5 release notes](https://github.com/bytecodealliance/wasmtime/releases/tag/v48.0.5).

## Tauri updater contracts — checked on 2026-10-02

Read in the published crates — `tauri-plugin-updater` 2.13.1 and
`tauri-plugin-opener` 2.7.0 (plugins-workspace commit `e5112843`), `tauri`
2.12.1, `tauri-cli` 2.12.1 and `tauri-bundler` 2.10.1 (tauri commit
`30da1fd6`) —, for [ADR-0051](adr/0051-automatic-updates-from-github-releases.md).
**To re-read at every bump of the updater plugin or of the CLI.**

| Fact | Consequence | Source |
|---|---|---|
| Static manifest: `version` (a leading `v` is trimmed), `notes`, `pub_date`, and `platforms` mapping a key to `url` and `signature`. Without a target set by the application, the key looked up is `{os}-{arch}-{installer}` then `{os}-{arch}`; `os` is `darwin` on macOS, `installer` is `app` for a `.app` or DMG, `appimage`, `deb`, `rpm` | `latest.json` carries `darwin-aarch64` and `linux-x86_64-appimage`; a bare `linux-x86_64` would also match a deb or rpm install | [updater.rs](https://github.com/tauri-apps/plugins-workspace/blob/e51128438011755f9e7277bad29b8c0978cf281c/plugins/updater/src/updater.rs), `RemoteRelease`, `get_urls`, `Installer::name`, `updater_os` |
| An endpoint whose scheme is not `https` is refused with `InsecureTransportProtocol` in a release build (a warning in debug), unless `dangerousInsecureTransportProtocol`. The check covers the endpoints only, not the `url` inside the manifest | The endpoint is HTTPS by construction; Oxyn refuses itself an archive URL that is not `https://github.com/…`; the archive's integrity rests on its signature | [config.rs](https://github.com/tauri-apps/plugins-workspace/blob/e51128438011755f9e7277bad29b8c0978cf281c/plugins/updater/src/config.rs), `validate_endpoints` |
| `UpdaterBuilder::timeout` applies to the check's request; the `Update` that `check` returns carries `timeout: None`, so `download` runs with no timeout unless `Update::timeout` is set | Oxyn's 30 s bound covers the check only; the download is ended by `cancel_update`, which aborts its task, or by the 60 s read timeout and HTTPS-only redirects set on the client through `UpdaterBuilder::configure_client` | updater.rs, `Updater::check` (the `Update { … timeout: None … }` literal), `Update::download` |
| `download` reads the whole archive into a `Vec<u8>`, then verifies the minisign signature against `pubkey` before returning it. The signature covers the archive's bytes and its trusted comment; the manifest is not signed | The bytes are kept in memory, already verified; whoever serves the manifest chooses which signed archive is offered | updater.rs, `Update::download`, `verify_signature` |
| `requireSignedVersion` (default `false`): after the signature check, the `version:` field of the trusted comment must equal the announced version, compared as semver; absent, the archive is refused (`MissingSignedVersion`), different, `SignedVersionMismatch` | Set to `true`: a forged manifest cannot pair a higher version with an older signed archive | config.rs, `require_signed_version`; updater.rs, `verify_signed_version` |
| The Tauri CLI writes `timestamp:…\tfile:…` in the trusted comment, and from 2.12.0 appends `\tversion:<app version>` when bundling (`sign_file(…, Some(settings.version_string()))`); 2.11.4 writes no version | The CLI is pinned to 2.12.1; an archive signed by an older CLI would be refused | [updater_signature.rs](https://github.com/tauri-apps/tauri/blob/30da1fd6e17de6107ecc850c95dfb16b5729f2dd/crates/tauri-cli/src/helpers/updater_signature.rs), `sign_file`; `crates/tauri-cli/src/bundle.rs` line 312 |
| `tauri signer generate -w <path>` prompts for a password when none is given, writes the private key there and the public key at `<path>.pub` | The runbook in [RELEASE](RELEASE.md#updater-signing-key) | `crates/tauri-cli/src/signer/generate.rs`, `helpers/updater_signature.rs`, `save_keypair` |
| `tauri signer sign` reads the key from `TAURI_SIGNING_PRIVATE_KEY` and its password from `TAURI_SIGNING_PRIVATE_KEY_PASSWORD`, writes `<file>.sig`, and with `--app-version <v>` calls the same `sign_file(…, Some(v))` as the bundler, hence the same `version:` field | Signing is a step of its own, after a build that never holds the key (`script/livraison signer`) | `crates/tauri-cli/src/signer/sign.rs`, `app_version` |
| The bundler signs updater artifacts only when `createUpdaterArtifacts` is on, at the end of `tauri build`/`tauri bundle`, with the key read from the environment of that process; a private key that does not match `plugins.updater.pubkey` only logs a **warning** | `createUpdaterArtifacts` stays off; the `manifeste` job verifies every pair against the committed public key | `crates/tauri-cli/src/bundle.rs`, `sign_updaters` (`signature.keynum() != public_key.keynum()` → `log::warn!`) |
| `patch_binary`, which writes the bundle type the plugin reads (`appimage`, `deb`…), runs for every package type unless `--no-binary-patching`, whether or not updater artifacts are created | The AppImage of a build without `createUpdaterArtifacts` is the one a signing build would produce | `crates/tauri-bundler/src/bundle.rs`, `bundle_project` |
| minisign **0.12** (GitHub release of 2025-01-15, the latest on 2026-10-02): `minisign-0.12-linux.tar.gz`, SHA-256 `9a599b48ba6eb7b1e80f12f36b94ceca7c00b7a5173c95c3efc88d9822957e73`, holds a static x86-64 binary at `minisign-linux/x86_64/minisign`; the archive verifies with its `.minisig` and the author's key `RWQf6LRCGA9i53mlYecO4IzT51TGPpvWucNSCh1CBM0QTaLn73Y7GFO3`, as printed in the project's README | The `manifeste` job installs it pinned by that checksum, like `cargo-about` | [release 0.12](https://github.com/jedisct1/minisign/releases/tag/0.12), [README](https://github.com/jedisct1/minisign/blob/master/README.md); checksum computed on download, signature checked with the macOS binary of the same release |
| `minisign -V -q -p <public key file> -m <file> -x <signature file>` accepts a `.sig` written by `tauri signer sign` 2.12.1 once decoded from Base64 (prehashed `ED` signatures) and prints its trusted comment; it exits non-zero for a modified file and for another key ("Signature key id … but the key id in the public key is …") | `script/livraison manifeste` verifies each archive this way before reading the signed version | run by hand on 2026-10-02, throwaway keys from `tauri signer generate` |
| A job's environment secrets reach only jobs that reference the environment; "Selected branches and tags" matches `GITHUB_REF` and is available for all public repositories, as are required reviewers; a workflow referencing an environment that does not exist creates it **without protection rules**; `gh secret set --env <name>` sets an environment secret | The signing secrets live in the `release` environment, `v*` tags only, created with its rules before the first release | [deployments and environments](https://docs.github.com/en/actions/reference/workflows-and-actions/deployments-and-environments), [manage environments](https://docs.github.com/en/actions/how-tos/deploy/configure-and-manage-deployments/manage-environments), [use secrets](https://docs.github.com/en/actions/how-tos/write-workflows/choose-what-workflows-do/use-secrets) |
| The repository is public; `GET /repos/so-keyldzn/oxyn/rulesets` now answers `[]` (it answered 403 while private, CI and GitHub delivery above) and it has no environment yet | A `v*` tag ruleset and the `release` environment can be created | GitHub API |
| The default comparison offers an update only if `release.version > current_version`; `allowDowngrades` (config only since 2.12.0, previously also settable by the webview's `check`) relaxes it to "different", and a custom comparator overrides both | Downgrades are refused by default; Oxyn sets neither | updater.rs, `check`; config.rs, `allow_downgrades`; plugin `CHANGELOG.md`, 2.12.0 |
| macOS install: the archive is unpacked into a temporary directory, the running `.app` renamed to a backup, the new one renamed into place. If the first rename fails with `PermissionDenied`, an AppleScript `do shell script … with administrator privileges` runs on the main thread and the install waits for it | No install on quit where the `.app`'s folder is not writable: the password prompt only follows a click | updater.rs, macOS `install_inner` |
| Linux install: an AppImage is replaced in place (a backup renamed beside it on the same device, restored on failure); a deb or rpm is installed with `dpkg -i` or `rpm -U` through `pkexec`, then a `zenity` or `kdialog` password prompt, then `sudo` | Oxyn disables updates for deb and rpm instead of running a package manager with elevation | updater.rs, `install_appimage`, `install_deb`, `install_rpm`, `try_tmp_locations` |
| The macOS updater archive is the `.app` produced by `bundle_project` — signed, notarized and stapled there — put in a `.tar.gz` by `bundle_update_macos`: `tar::Builder` with `follow_symlinks(false)`, `append_dir_all("<App>.app", …)`, gzip. The AppImage, deb and rpm are signed as they are. The plugin's macOS install drops the first component of every entry (`skip(1)`) | `script/apple-release build` writes the same archive with Python's `tarfile` (links kept, the `.app` at the root), so that no build needs the key; `script/apple-release verify` checks the `.app` inside it | [macos/app.rs](https://github.com/tauri-apps/tauri/blob/30da1fd6e17de6107ecc850c95dfb16b5729f2dd/crates/tauri-bundler/src/bundle/macos/app.rs), [bundle.rs](https://github.com/tauri-apps/tauri/blob/30da1fd6e17de6107ecc850c95dfb16b5729f2dd/crates/tauri-bundler/src/bundle.rs), [updater_bundle.rs](https://github.com/tauri-apps/tauri/blob/30da1fd6e17de6107ecc850c95dfb16b5729f2dd/crates/tauri-bundler/src/bundle/updater_bundle.rs) |
| `AppHandle::restart()` never returns: on the main thread it relaunches at once, skipping the exit events; elsewhere it requests the exit and sleeps forever. `request_restart()` sets the restart flag and requests an exit with `RESTART_EXIT_CODE`, which goes through `ExitRequested` and `Exit`, then relaunches | "Restart now" calls `request_restart()` after the ordered shutdown | [app.rs](https://github.com/tauri-apps/tauri/blob/30da1fd6e17de6107ecc850c95dfb16b5729f2dd/crates/tauri/src/app.rs), `restart`, `request_restart` |
| `tauri build` stops when the `tauri` crate and `@tauri-apps/api` differ in major or minor version, unless `--ignore-version-mismatches` | The two move together | `crates/tauri-cli/src/build.rs`, `info/plugins.rs`, `check_mismatched_packages` |
| `open_url` calls the `open` crate's `that_detached`; `open` 5.4.4 tries, on Linux, `xdg-open`, `gio open`, `gnome-open`, `kde-open` (and WSL first under WSL), prefixes an argument starting with `-` with `./`, and spawns detached | The release page is opened by the free function `open_url`, from Rust, without registering the plugin | [open.rs](https://github.com/tauri-apps/plugins-workspace/blob/e51128438011755f9e7277bad29b8c0978cf281c/plugins/opener/src/open.rs); `open` 5.4.4 `src/unix.rs` |
| "Get the latest release": "the most recent non-prerelease, non-draft release, sorted by the `created_at` attribute"; `/releases/latest/download/<asset>` links to an asset of the latest release | A draft is never offered; publishing is what offers the update | [REST releases](https://docs.github.com/en/rest/releases/releases#get-the-latest-release), [linking to releases](https://docs.github.com/en/repositories/releasing-projects-on-github/linking-to-releases) |

## GPUI

| Fact | Value | Source | Checked on |
|---|---|---|---|
| Latest published version | `0.2.2`, published on 2025-10-22 | crates.io API | 2026-09-05 |
| License | Apache-2.0 | crates.io API | 2026-09-05 |
| Edition | 2024 | crates.io API | 2026-09-05 |
| Declared MSRV | **none** (`rust_version` absent) | crates.io API | 2026-09-05 |
| Normal non-optional dependencies | 65 | crates.io API `/dependencies` | 2026-09-05 |
| Features | `default`, `inspector`, `leak-detection`, `macos-blade`, `runtime_shaders`, `screen-capture`, `test-support`, `wayland`, `windows-manifest`, `x11` | crates.io API | 2026-09-05 |

> **Two verified traps.**
> 1. The latest release is **nearly eleven months** old while development
>    continues in the upstream repository. The pinned version will receive
>    neither fixes nor new APIs. It is an accepted cost, settled in
>    [ADR-0009](adr/0009-source-dependance-gpui.md).
> 2. `gpui` pins several of its dependencies with `=` — including
>    `cocoa =0.26.0`, `cocoa-foundation =0.2.0`, `core-foundation =0.10.0`.
>    An Oxyn dependency on another version of these crates does not resolve:
>    Cargo fails instead of unifying. To check before adding any crate that
>    touches macOS system APIs.

### The GPUI test harness

Read in the sources of `gpui 0.2.2` as published on crates.io — the
`test-support` feature is not documented on docs.rs, which builds with the
default features.

| Fact | Value | Source | Checked on |
|---|---|---|---|
| Feature to enable | `test-support` — pulls `leak-detection`, `rand`, `collections/test-support`, `util/test-support`, `http_client/test-support`, `wayland`, `x11` | `Cargo.toml` of the published crate | 2026-09-07 |
| Real cost on macOS | `wayland` and `x11` are declared under `[target.'cfg(any(target_os = "linux", target_os = "freebsd"))'.dependencies]`: only adds `rand` and `backtrace` | `Cargo.toml`, `[target.…]` sections | 2026-09-07 |
| Macro | `#[gpui::test]`, re-exported from `gpui_macros` | `src/gpui.rs:81` | 2026-09-07 |
| Contexts | `TestAppContext`, `VisualTestContext` | `src/app/test_context.rs` | 2026-09-07 |
| Simulation | `draw`, `simulate_click`, `simulate_mouse_down/up/move`, `simulate_keystrokes`, `simulate_input`, `simulate_modifiers_change`, `simulate_resize`, `simulate_prompt_answer`, `dispatch_action`, `run_until_parked` | `src/app/test_context.rs` | 2026-09-07 |
| Platform | `TestPlatform` — no window, no GPU, no display server required | `src/platform/test/platform.rs` | 2026-09-07 |
| Text system | `NoopTextSystem` — fake font: `advance = 600 × glyph_id`, `glyph_id = ch.len_utf16()`, `rasterize_glyph` returns an empty buffer | `src/platform.rs:594` | 2026-09-07 |

> **What the harness does not measure.** `NoopTextSystem` makes text metrics
> deterministic and wrong, and no pixel is produced. So: no image capture, no
> rendering comparison, and **no valid assertion on a dimension that depends on
> the width of a text**. Such a test is green whatever the real interface.
> Consequence for tests:
> [tests.md](../.claude/rules/tests.md#interface-tests).

## Figma interface resources

Sources checked on **2026-09-07** during the GPUI integration:

| Resource | Frozen source | Use |
|---|---|---|
| Hugeicons Stroke Rounded | [Source repository](https://github.com/hugeicons/hugeicons-static/tree/f9dbcca8d72cc2777a0ccd873c274d9bf7a153e6), outlines exported from the [Oxyn Figma](https://www.figma.com/design/Yviemi4brBczzdRdBp1ONv/Oxyn?node-id=13-291) | Twelve 16 × 16 px SVGs, exact bytes embedded; upstream notice kept |
| Oxyn brand | Same Figma, component `149:22199`, re-read after the color update | Two 32 × 32 px SVGs depending on the theme, inner margins kept; orange fragment at the source, recolored verdigris on 2026-09-23, paths unchanged ([brand](../assets/brand/README.md#colors)) |
| Geist | [vercel/geist-font](https://github.com/vercel/geist-font/tree/10dc7658f13c38a474cde201bb09a4617267545b/fonts/Geist/ttf) | Regular, Medium and SemiBold, TTF embedded under SIL OFL |

Nodes, dimensions and SHA-256: icons in `assets/ui/provenance.json` and
fonts in `assets/fonts/provenance.json`, removed from the repository, readable at commit `8a1b7ff`. The license notices stay with
the resources. The files are not loaded from Figma at startup:
`UiAssets` returns the bytes included at compile time and the fonts are
registered before the window opens.

## Candidate crates

Read at the registry, not yet adopted. None enters the repository without
going through [`/adr`](../.claude/commands/adr.md) if it commits the architecture.

| Crate | Latest stable | Published on | Checked on |
|---|---|---|---|
| `tokio` | `1.53.1` | 2026-07-20 | 2026-09-05 |
| `sqlx` | `0.9.0` | 2026-05-21 | 2026-09-05 |
| `rusqlite` | `0.40.2` | 2026-08-08 | 2026-09-05 |
| `duckdb` | `1.10505.0` | 2026-07-22 | 2026-09-05 |
| `mongodb` | `3.9.0` | 2026-09-03 | 2026-09-05 |
| `redis` | `1.6.0` | 2026-08-15 | 2026-09-05 |
| `serde` | `1.0.229` | 2026-07-18 | 2026-09-05 |
| `thiserror` | `2.0.20` | 2026-08-08 | 2026-09-05 |
| `anyhow` | `1.0.104` | 2026-07-18 | 2026-09-05 |
| `tracing` | `0.1.44` | 2025-12-18 | 2026-09-05 |
| `criterion` | `0.8.2` | 2026-02-04 | 2026-09-10 |
| `gpui` | `0.2.2` | 2025-10-22 | 2026-09-05 |

> `gpui` appears here to be covered by the automatic checker; it is adopted,
> not a candidate — see the GPUI section above and
> [ADR-0009](adr/0009-source-dependance-gpui.md).

> `criterion` has been adopted since 2026-09-10, as **`[dev-dependencies]`
> only** — see the "Benchmarks" section below. It stays in this table to be
> covered by the automatic checker.

> `duckdb` versions follow DuckDB's upstream version (`1.10505.0`), not
> classic Rust semver. Do not infer an API break from a major jump.

## Codex — extended context

Checked on **2026-09-07** for the local configuration
[.codex/config.toml](../.codex/config.toml).

| Fact | Value | Source | Checked on |
|---|---|---|---|
| Model and advertised maximum window | `gpt-6-astra`, 1,050,000 tokens | [Official page](https://developers.openai.com/api/docs/models/gpt-6-astra) | 2026-09-07 |
| Context settings | `model_context_window` and `model_auto_compact_token_limit` | [Official reference](https://learn.chatgpt.com/docs/config-file/config-reference) | 2026-09-07 |
| Local loading | `.codex/config.toml`, only for an approved project; launch options take precedence | [Official configuration](https://learn.chatgpt.com/docs/config-file/config-basic) | 2026-09-07 |
| Client installed during the check | `codex-cli 0.153.4` | `codex --version` | 2026-09-07 |
| Local catalog observed before override | 272,000 tokens, 95 % usable; existing session at 258,400 tokens | `~/.codex/models_cache.json`, `token_count` event of the session | 2026-09-07 |

Project choice: effort `high`, declared window of 1,050,000 tokens and
compaction at 700,000 tokens to keep a margin for responses, reasoning and
tool outputs. This threshold is a local choice, not an official limit. The
effective window depends on the client and the service; writing this value
does not prove that a request of that size was accepted. With the observed
local reserve of 5 %, the expected usable window is 997,500 tokens.

For API-billed usage, the model page announces, beyond 272,000 input tokens,
a multiplier of 2 on input and cache, and of 1.5 on output for the whole
request. Do not extrapolate these prices to the quotas of a ChatGPT
subscription.

Additional settings checked on **2026-09-07**:

| Fact | Local decision | Source |
|---|---|---|
| Prompt caching enabled by default on compatible models | Let the service manage the cache; no top-level `prompt_cache`, `prompt_cache_key` or `prompt_cache_retention` key documented for the Codex file | [Cache API](https://developers.openai.com/api/docs/guides/prompt-caching), [Codex reference](https://learn.chatgpt.com/docs/config-file/config-reference) |
| `web_search = "live"` enables live web search | Check current sources in line with I-12; the `cached` web mode is independent of prompt caching | [Codex reference](https://learn.chatgpt.com/docs/config-file/config-reference) |
| `tui.status_line` configures the CLI bar | Show `model-with-reasoning`, `context-remaining`, `git-branch` | [Official example](https://learn.chatgpt.com/docs/config-file/config-sample) |

The cache reuses identical prefixes: keeping instructions stable and
continuing a task in its own thread favors that reuse, without guaranteeing
it. A cache hit reduces processing work; it does not remove tokens from the
context window. The retention and routing parameters documented for the API
must not be transposed into invented Codex keys.

## Codex — local agents and MCP

Checked on **2026-09-07** for [.codex/](../.codex/README.md).

| Fact | Local decision | Source |
|---|---|---|
| Project agents are discovered in `.codex/agents/*.toml`; `name`, `description` and `developer_instructions` are required | Eleven short profiles point to the shared guides and to the adaptations of AGENTS.md | [Custom agents](https://learn.chatgpt.com/docs/agent-configuration/subagents#custom-agents) |
| Omitted model and effort settings inherit from the launch context | No model override in the profiles | [Subagents](https://learn.chatgpt.com/docs/agent-configuration/subagents) |
| `agents.max_concurrent_threads_per_session` bounds simultaneous subagents, excluding the main agent | Three subagents at most; local choice, not a service limit | [Configuration reference](https://learn.chatgpt.com/docs/config-file/config-reference) |
| A profile can declare `sandbox_mode`, but the parent's active overrides may take precedence | `read-only` default for the four reviewers; also keep the instruction not to modify anything | [Subagent permissions](https://learn.chatgpt.com/docs/agent-configuration/subagents#approvals-and-sandbox-controls) |
| `mcp_servers.<id>.required = false` leaves the server optional at startup | Keep the two pre-existing Figma declarations without requiring their availability | [Configuration reference](https://learn.chatgpt.com/docs/config-file/config-reference) |

Delegation remains subject to the request and to the instructions of
AGENTS.md. The profiles install no Claude hook and do not change the global
configuration. Figma availability and authentication must be checked in the
session that uses the service.

## MCP ecosystem

| Fact | Value | Source | Checked on |
|---|---|---|---|
| Maintained reference servers | `everything`, `fetch`, `filesystem`, `git`, `memory`, `sequential-thinking`, `time` | `github.com/modelcontextprotocol/servers` | 2026-09-05 |
| **Archived** reference servers | `postgres`, `sqlite`, `github` | same | 2026-09-05 |

> Direct consequence: there is **no** official MCP server for PostgreSQL or
> SQLite. Any database server plugged into Oxyn would be a third-party server,
> to audit. The full reasoning is in [MCP.md](MCP.md).

## Figma mockup tokens

File `Yviemi4brBczzdRdBp1ONv`, collection `Oxyn / Primitives`. Read through
the local Figma Dev Mode MCP server (`get_metadata`, `get_variable_defs`), not
copied from a screenshot. Colors and icons have their own, more detailed
provenance in `assets/ui/provenance.json`, removed from the repository, readable at commit `8a1b7ff`.

Dimensions, spacings and radii are published in
`crates/oxyn-ui/src/theme.rs` — `Metrics`, `Spacing`, `Radii` — where each
field cites its node. **Typography only partially is**: `Typography` carries
neither weights nor named styles, and two of its values remain unsourced —
see the table of gaps below.

| Fact | Value | Source | Checked on |
|---|---|---|---|
| Spacing scale | `space/0`=0, `space/4`=4, `space/8`=8, `space/12`=12, `space/16`=16, `space/24`=24 | Nodes `13:291` and `47:8222` | 2026-09-07 |
| Corner radii | `radius/6`=6, `radius/8`=8, `radius/full`=999 | same | 2026-09-07 |
| Toolbar height | `48` | Node `47:8422` "Connection toolbar", confirmed by `190:1543`, `229:7640`, `232:9103` | 2026-09-15 |
| Width of the expanded side panel | `280` | Nodes `8:4`, `13:292` | 2026-09-07 |
| Width of the collapsed side panel | `64` | Nodes `13:165`, `13:484` | 2026-09-07 |
| Height of a control | `38` | Nodes `47:8461`, `47:8465`, `47:8469` | 2026-09-07 |
| Families and weights | Geist — Title 24/32 SemiBold, Body 13/20 Regular, Label 13/20 Medium, Caption 11/16 Regular, Section 11/16 Medium | Nodes `13:291`, `47:8222` | 2026-09-07 |

> **Correction of 2026-09-15 — the toolbar height was `52`.**
> Node `47:8422` had been read under the name "Workspace toolbar" on
> 2026-09-07; re-read from the server, it is called "Connection toolbar" and
> measures **48**. The three workspace boards confirm it without exception.
> The `52` therefore came from no frame at all.
>
> What makes the case instructive, and is exactly the failure mode
> [I-12](../CLAUDE.md#i-12) describes: the repository had everything needed to
> notice it — the value was documented, sourced, dated, and a test of
> `theme.rs` anchored it. The test was green, on a wrong value. Nothing turned
> red because **the field had no reader**: the connection bar drew itself with
> a `48` hard-coded in the view. Two sources for the same measure, one wrong
> and the other invisible to the theme. The view now reads
> `Metrics::toolbar_height`.

### Embedded binaries and their licenses

These files are linked into the binary by `include_bytes!` in
`crates/oxyn-ui/src/icons.rs`. They carry an exact upstream commit, like a
code dependency.

| Fact | Value | Source | Checked on |
|---|---|---|---|
| Interface font | Geist Regular, Medium, SemiBold, commit `10dc7658f13c38a474cde201bb09a4617267545b` | [vercel/geist-font](https://github.com/vercel/geist-font); SHA-256 in `assets/fonts/provenance.json`, removed from the repository, readable at commit `8a1b7ff` | 2026-09-07 |
| Font license | SIL Open Font License 1.1 | `assets/fonts/OFL.txt`, `assets/fonts/LICENSE.txt` | 2026-09-07 |
| Icons | Hugeicons Stroke Rounded, 12 glyphs, commit `f9dbcca8d72cc2777a0ccd873c274d9bf7a153e6` | [hugeicons/hugeicons-static](https://github.com/hugeicons/hugeicons-static); SHA-256 in `assets/ui/provenance.json`, removed from the repository, readable at commit `8a1b7ff` | 2026-09-07 |
| Icon license | **No MIT license granted**; the upstream README allows use as is, without mentioning a redistribution right | `assets/ui/HUGEICONS-UPSTREAM-README.txt`, removed from the repository, readable at commit `8a1b7ff` | 2026-09-07 |

> **To settle before the first binary release.** Oxyn redistributes these
> twelve glyphs by linking them into the executable. As long as no version is
> published, the question does not arise; it will arise all at once on the day
> of the first release, and it is a legal question, not a code one.

### What could not be read, and why

The Dev Mode server applies a **daily quota**, exhausted on 2026-09-07 by a
sweep of page identifiers. The following therefore remained unverified, and
must not be considered sourced until they are re-read:

| Not read | Where to look for it | Consequence in the code |
|---|---|---|
| Status bar height | pages `03 · Foundations` or `22 · Database workspace` | `Metrics::status_bar_height` is 32, a placeholder value **not confirmed** |
| Spacings 20 and 32 | same | **not published**: absent from the two screens read, which only use 0/4/8/12/16/24 |
| Grid metrics — row and header height, column widths, gutter | page `22 · Database workspace` | the pre-existing values of `Metrics` are kept as they are, without being attributed to the mockup |
| Focus ring thickness | component `Focus 11:69` | `Metrics::focus_ring` is 2, justified by legibility and not by the mockup |
| Line height of the monospace editor | page `03 · Foundations` | `Typography::line_height` is 18: neither 20 nor 16, the two line heights read; it is an editor value, not to be "fixed" from the typography row above |
| Monospace family | same | `Typography::mono_family` is `Menlo`, a macOS system font; the mockup could not be consulted on this point |

The identifiers of the pages `03 · Foundations` and `22 · Database workspace`
were not found: `get_metadata` requires a known node, and pages cannot be
enumerated. Getting them requires opening them in the Figma application, or
reading the `?node-id=` URL of each.

## PostgreSQL previews and Arrow time zones

Checked on 2026-09-07 in the sources resolved by `Cargo.lock` and in the
upstream sources:

- [Arrow: time zones](https://docs.rs/arrow-array/latest/arrow_array/timezone/struct.Tz.html):
  without the `chrono-tz` feature, only fixed offsets are accepted. The driver
  provides `UTC` for `timestamptz`; the workspace therefore enables this
  feature for the grid and the exports. Cargo resolves the transitive
  dependency [chrono-tz 0.10.4](https://crates.io/crates/chrono-tz/0.10.4),
  without changing the Arrow version.
- [PostgreSQL: pg_type](https://www.postgresql.org/docs/17/catalog-pg-type.html):
  `typsend = 0` means there is no binary output; category `Z` designates
  internal types. The installed SQLx category decoder refuses `Z`, which
  notably blocks `pg_node_tree` before the rows are even read.
- [PostgreSQL: OID aliases](https://www.postgresql.org/docs/17/datatype-oid.html):
  the text output of `regproc` and of the other aliases exposes object names.
  The preview asks the server for this rendering; binary bytes that happen to
  be valid UTF-8 are not a reliable text representation.

## `rustls` — security advisory of 2026-09-14

`cargo deny` reported **RUSTSEC-2026-0285** on `rustls 0.23.43`, pulled by
`reqwest 0.13.4` through `hyper-rustls 0.27.9` for `oxyn-llm`.

The advisory: rustls accepted TLS 1.3 handshake messages sent at the **wrong
encryption level** when they followed a key-changing message in the same
record — a plaintext `EncryptedExtensions` packed with the `ServerHello`, for
example. RFC 8446 §5.1 requires terminating the connection with an
`unexpected_message` alert. Same functional defect as
[GO-2026-4340](https://pkg.go.dev/vuln/GO-2026-4340) (CVE-2025-61730).

**Real scope, as the advisory describes it**: the handshake transcript stays
authenticated, so an attacker in a network position can neither alter nor
complete it. The practical effect is that a peer could send in plaintext
messages that should have been encrypted without rustls refusing the
connection.

Fixed by `cargo update -p rustls`: **0.23.43 → 0.23.45**, a patch update
within the same semantic range. No manifest changed, only `Cargo.lock`.
Checked on 2026-09-14: `cargo deny check advisories` reports nothing any more.

## Agent Client Protocol — check of 2026-09-14

Lead opened by the user: the AI integration, above all to avoid going through
the APIs; "there are two modes, one with an API and the other with external
agents". Checked at the sources, not from memory.

**The protocol.** The Agent Client Protocol is **JSON-RPC**, over `stdio` for
a local agent, over HTTP or WebSocket for a remote agent. A local agent is a
**child process of the editor**. The dialogue turn is documented with its
methods: `session/prompt` (client → agent) opens the turn, `session/update`
(agent → client) streams the fragments — `agent_message_chunk`, `plan`,
`tool_call`, `tool_call_update`, `usage_update` —, `session/request_permission`
(**agent → client**) asks for authorization before running a tool, and
`session/cancel` (client → agent) interrupts. The turn ends with a response
carrying a `StopReason`: `end_turn`, `max_tokens`, `max_turn_requests`,
`refusal` or `cancelled`. Consulted [agentclientprotocol.com](https://agentclientprotocol.com/protocol/prompt-turn)
on 2026-09-14.

**The library.** `agent-client-protocol` **2.1.0** on crates.io, published on
2026-09-04, under **Apache-2.0** — a license already accepted by `deny.toml`.
Declared `rust-version` **1.88.0**, edition **2024**: compatible with the
repository's pinned toolchain (1.98.1) without touching it. Repository:
[agentclientprotocol/rust-sdk](https://github.com/agentclientprotocol/rust-sdk).
The SDK exposes the `Client`, `Agent`, `Proxy` and `Conductor` roles with
connection builders; **the major version number does not tell the protocol
version** — 2.1.0 carries the *stable* v1 protocol and a *draft* v2, the latter
behind `.v2()`. The HTTP/SSE and WebSocket transports live in a separate
crate, `agent-client-protocol-http`, which we would not need for a local
agent.

**What happens to the child process on cancellation** — read in the crate's
source on **2026-09-15**, because the whole cancellation of an agent turn
depends on it and the claim was going around without proof. In
`agent-client-protocol-2.1.0/src/acp_agent.rs`:

| Fact | Where |
|---|---|
| The child is launched **as leader of its own group** (`std_cmd.process_group(0)`) — killing the group therefore does not reach Oxyn | `spawn_process` |
| `ChildGuard::terminate` sends `SIGKILL` to the **group** (`rustix::process::kill_process_group`) then `kill()` as a fallback, which reaches the grandchildren of an `npx` or `uvx` launcher | `ChildGuard` |
| The guard is built **before the first `poll`**: "Create the guard eagerly so cancelling this connection before the monitor is first polled still terminates the whole process group" | at the creation of `child_wait` |

Consequence retained: **dropping the conversation future is enough** to
terminate the agent, including if the cancellation arrives before anything was
read. This is what `run_turn` relies on, selecting the conversation against
the cancellation token rather than re-reading a flag between two steps.

**What a client editor does with it.** An external agent is declared there by
a command, its arguments and its environment, and the editor launches it as a
separate process. **No API key is required for an external agent**, which
carries its own authentication; billing, terms and data retention are a
matter between the user and the agent's provider. This is in contrast with
native providers, where the key is configured in the editor. Read on
2026-09-14.

**What the crate adds to the process**, measured on 2026-09-14 by
`cargo tree -p oxyn-ai --edges normal -i <crate>`: `async-io 2.6.0`,
`async-process 2.5.0`, `async-signal 0.2.14` and `blocking 1.7.0`. Tokio is
only a **dev** dependency of the crate — its core is `futures`, runtime
agnostic. On the other hand `async-io` starts a reactor thread and `blocking`
a pool: **two reactors coexist** with Tokio's. `smol`, `async-executor` and
`async-global-executor` appear in `Cargo.lock` but **not** in the normal graph
of `oxyn-ai` — they come from dev dependencies elsewhere, and the distinction
is worth making: reading `Cargo.lock` alone would have led to the conclusion
of one more full runtime.

The crate's `ConnectTo` path installs a guard that terminates the **process
group** (`process_group(0)` on Unix), not only the child: an agent distributed
behind `npx` or `uvx` would otherwise re-attach to pid 1 and would not stop
reliably on EOF of its standard input.

What the repository concludes from this is decided in
[ADR-0026](adr/0026-agents-externes-acp.md), not here.

### ACP adapters for Claude Code and Codex — check of 2026-09-15

User request: connect to Claude through **Claude Code already installed and
authenticated** (their subscription, no key entrusted to Oxyn), and the same
for **Codex**. Neither speaks ACP natively: each goes through an adapter. Read
from the npm registry, the ACP registry, the adapters' source and the official
documentation, on **2026-09-15**; these are the values of
`crates/oxyn-ai/src/external/presets.rs`.

| Package | Version | License | `bin` | Source |
|---|---|---|---|---|
| `@agentclientprotocol/claude-agent-acp` | **0.78.0** | Apache-2.0 (the ACP registry says "proprietary", see below) | `claude-agent-acp`; `engines`: `node >=22` | [npm registry](https://registry.npmjs.org/@agentclientprotocol/claude-agent-acp), [ACP registry](https://github.com/agentclientprotocol/registry/blob/main/claude-acp/agent.json) |
| `@anthropic-ai/claude-agent-sdk` (dependency, embedded CLI 2.1.270) | 0.3.270 | "SEE LICENSE IN README.md" | — | [npm registry](https://registry.npmjs.org/@anthropic-ai/claude-agent-sdk/0.3.270) |
| `@agentclientprotocol/codex-acp` | **1.12.0** | Apache-2.0 | `codex-acp`; no `engines` declared, but its dependency `open@^11` requires `node >=20`, the highest of its direct dependencies (`vscode-jsonrpc@9`: `>=14`, `diff@9`: `>=0.3.1`, `zod@4` and `@agentclientprotocol/sdk@1.4`: nothing) — read from the npm registry on 2026-09-23 | [npm registry](https://registry.npmjs.org/@agentclientprotocol/codex-acp), [ACP registry](https://github.com/agentclientprotocol/registry/blob/main/codex-acp/agent.json) |
| `@openai/codex` (dependency) | 0.154.0 | Apache-2.0 | `codex`; `node >=16` | [npm registry](https://registry.npmjs.org/@openai/codex/latest) |

Both adapters publish several times a week (their two latest versions date
from 2026-09-15, seven minutes apart): the proposed command **pins** the
version, `npx -y <package>@<version>`, the form declared by the ACP registry.

**Sign-in.** Read in the source of each adapter (`src/acp-agent.ts`,
`src/CodexAuthMethod.ts`) and in each agent's documentation:

- Claude: the adapter announces **no** method if the client does not declare
  `clientCapabilities.auth.terminal`; otherwise it announces `terminal`
  methods (`--cli auth login --claudeai`). The specification reserves this
  capability for the client that "can reproduce the configured agent
  invocation in an interactive terminal": Oxyn cannot and **does not declare
  it**. Without a session, it returns `auth_required` (-32000). Sign-in
  happens in a terminal through `claude auth login`
  ([CLI reference](https://code.claude.com/docs/en/cli-reference));
  credentials live in the macOS keychain or `~/.claude`
  ([authentication](https://code.claude.com/docs/en/authentication)), which
  the adapter re-reads. **Inferred, not written** in the adapter's
  documentation: a sign-in made with the user's `claude` is reused with an
  identical configuration.
  **Without `claude` installed — checked on 2026-09-23** in the published
  source of 0.78.0 (`npx` cache): `dist/index.js` passes everything after
  `--cli` to the CLI that the SDK embeds (`claudeCliPath()`,
  `dist/acp-agent.js`), and the `terminal` methods it announces are exactly
  `--cli auth login --claudeai` and `--cli auth login --console`, appended to
  the command that launches it. Measured on the development machine:
  `npx -y @agentclientprotocol/claude-agent-acp@0.78.0 --cli --version`
  answers `2.1.270 (Claude Code)`, and `… --cli auth login --help` lists
  `--claudeai` ("Use Claude subscription (default)"). Oxyn therefore proposes
  this command, composed with the declared command, when `claude` cannot be
  found on the providers screen, and always in the panel for the pinned
  declaration (it does not look for `claude` there). Nothing equivalent is
  verified for `codex-acp`: `codex login` remains the only proposal.
- Codex: methods of kind `agent`, which go through `authenticate` —
  `chat-gpt` (succeeds immediately if an account is already signed in, opens
  the browser otherwise) and `api-key` (key passed in `_meta` or read from the
  environment, which Oxyn **does not offer**: it holds no agent key).
  `session/new` returns `auth_required` without an account; terminal sign-in
  is `codex login` ([Codex authentication](https://developers.openai.com/codex/auth)).

**Usual locations** searched when the providers screen opens (and by "Detect
again"), because an application launched from the Finder does not inherit the
shell's `PATH`:
`~/.local/bin` (native installers of Claude Code and Codex,
[Claude Code installation](https://code.claude.com/docs/en/setup),
[Codex install script](https://raw.githubusercontent.com/openai/codex/main/scripts/install/install.sh)),
`~/.claude/local` (old local npm installation of Claude Code),
`/opt/homebrew/bin`, `/usr/local/bin`, `/home/linuxbrew/.linuxbrew/bin`
([Homebrew](https://docs.brew.sh/Installation)). Only **absolute** `PATH`
entries are kept: `.` or `bin` would resolve against Oxyn's current
directory.

**nvm and Volta — check of 2026-09-23.** Added after a machine whose Node comes
from nvm did not find `npx` from the Finder.

| Manager | What is read | Source |
|---|---|---|
| nvm **0.40.8** | installed in `~/.nvm`, or `${XDG_CONFIG_HOME}/nvm` if that variable exists (`NVM_DIR`) | [README](https://github.com/nvm-sh/nvm/blob/v0.40.8/README.md), l. 120-126 |
| | a version lives in `$NVM_DIR/versions/node/<version>`, its executable in `<version>/bin` | [`nvm.sh`](https://github.com/nvm-sh/nvm/blob/v0.40.8/nvm.sh): `nvm_version_dir` (l. 781-785), `NVM_NODE_PATH="${VERSION_PATH}/bin/…"` (l. 253) |
| | aliases are files of `$NVM_DIR/alias` (`nvm_alias_path`, l. 796), LTS ones under `alias/lts/`; an alias can point to another alias, nvm follows the chain and stops on a cycle (`nvm_resolve_alias`, l. 1553); `..` is refused in a name (l. 1507-1510) | `nvm.sh` |
| | `default` can be `node` (the most recent installed), `18` (the most recent v18.x), `18.12` (the most recent v18.12.x) — "The first version installed becomes the default" | README, l. 396 and 626-628 |
| Volta | "The shim directory is at `$VOLTA_HOME/bin`", `VOLTA_HOME` being `~/.volta` on Unix | [Volta installers](https://docs.volta.sh/advanced/installers) |

What Oxyn does with it (`crates/oxyn-ai/src/external/locate/nvm.rs`): it
follows the `default` alias under `~/.nvm` and keeps the installed version it
designates **if it meets the agent's minimum** — `node >=22` for
`claude-agent-acp`, `node >=20` for `codex-acp` (above), none for an agent
Oxyn does not know; otherwise the highest installed version that meets it;
otherwise nothing. `NVM_DIR`, `XDG_CONFIG_HOME` and `VOLTA_HOME` are not read:
a shell profile sets them, and a process that ran one already has these
directories in its `PATH`. **Not verified, hence not searched**: fnm (its
`multishell` directories are specific to each shell) and asdf.

**Gap to report.** The ACP registry declares Claude Agent's license
"proprietary" whereas `package.json` and `LICENSE` say Apache-2.0; the likely
explanation is the `@anthropic-ai/claude-agent-sdk` dependency, under
Anthropic's terms. Oxyn redistributes neither: `npx` downloads them onto the
user's machine.

### Exposing Oxyn's tools to an external agent — check of 2026-09-16

Question: through which transport can an external agent reach Oxyn's tools
([ADR-0030](adr/0030-outils-oxyn-exposes-a-un-agent-externe.md))?

**What the crate offers.** `agent-client-protocol` 2.1.0 declares four MCP
server transports in `NewSessionRequest.mcp_servers`
(`agent-client-protocol-schema-1.7.0/src/v1/agent.rs:2628`): `Stdio` — "All
Agents MUST support this transport" —, `Http`, `Sse`, and `Acp`. Only `Acp`
carries the server **in memory**, with no process and no port; it is behind the
`unstable_mcp_over_acp` feature and conditioned on a capability announced by
the agent (`McpCapabilities.acp`, same file, line 4537).

**Measurement, not assumption.** Both adapters, as installed on the
development machine, were queried with an ACP `initialize` — without
authentication or access to a database. What they announce:

| Adapter | Measured version | `mcpCapabilities` |
|---|---|---|
| `@agentclientprotocol/claude-agent-acp` | 0.78.0 | `{"http": true, "sse": true}` — `acp` **absent** |
| `@agentclientprotocol/codex-acp` | 1.12.0 | `{"acp": false, "http": true, "sse": false}` |

**Consequence: the `Acp` transport is unusable today**, and the only transport
accepted by both agents besides `stdio` is **`http`**. `McpServerHttp` carries
`name`, `url` and `headers` (`.../schema-1.7.0/src/v1/agent.rs:2667`), hence a
bearer token.

The measurement also confirms the versions read on 2026-09-15: the cached
binaries do declare 0.78.0 and 1.12.0, and Codex announces the `api-key` and
`chat-gpt` authentication methods, as documented above.

**The MCP protocol is not in the crate.** `agent-client-protocol` 2.1.0
contains neither `initialize`, nor `tools/list`, nor `tools/call`:
`McpToolRegistry` provides the catalog and the schemas, nothing serves them on
the wire. The adaptation crate is named in its own sources
(`agent-client-protocol-2.1.0/src/mcp_server/mod.rs:18`):

| Crate | Version | License | Published | Requires |
|---|---|---|---|---|
| `agent-client-protocol-rmcp` | **3.1.0** | Apache-2.0 | 2026-09-04 | `agent-client-protocol ^2.1.0`, `rmcp ^2.1.0`, `tokio ^1.52`, `tokio-util ^0.7`, `schemars ^1.0` |

Read at [crates.io](https://crates.io/api/v1/crates/agent-client-protocol-rmcp)
on 2026-09-16.

**It is not retained.** The `Acp` transport it serves is the one the agents do
not accept (measurement above), and over an HTTP transport the MCP protocol is
ours to handle anyway. Recording it here keeps the question from being asked
again without the measurement.

**What the bridge uses instead**, read at crates.io on **2026-09-16**.
`hyper`, `hyper-util` and `http-body-util` were already in `Cargo.lock` through
`reqwest` and `tauri`: declaring them brings no new family into the tree.

| Crate | Version | License | Why |
|---|---|---|---|
| `hyper` | **1.11.1** | MIT | HTTP listening of the MCP server, on the loopback |
| `hyper-util` | **0.1.20** | MIT | the service and accepting connections |
| `http-body-util` | **0.1.5** | MIT | reading and writing a full body |
| `async-process` | **2.5.0** | Apache-2.0 OR MIT | launching the agent with an allow-listed environment; its streams are already `futures::io`, so no executor bridge |
| `rustix` | **0.38.44** | Apache-2.0 WITH LLVM-exception OR Apache-2.0 OR MIT | killing the process **group**. The latest version is `1.1.4`; we align on `0.38.44`, the one `agent-client-protocol` already pulls, so as not to compile two copies of a system-call crate |

**The MCP revisions the bridge announces**, read on **2026-09-16** in the
package installed by the Claude adapter, and not from memory. The server
**negotiates** them: it keeps the revision requested by the agent when it is in
the list, and answers the most recent otherwise
(`crates/oxyn-ai/src/external/mcp.rs`, `SUPPORTED_VERSIONS`).

| Source | Version | License | Where |
|---|---|---|---|
| `@modelcontextprotocol/sdk` | **1.30.0** | MIT | `dist/esm/types.js:4` and `dist/cjs/types.js:33-35`, in the `npx` cache of `@agentclientprotocol/claude-agent-acp` 0.78.0; *peer* dependency `^1.29.0` of `@anthropic-ai/claude-agent-sdk` 0.3.270 |

`LATEST_PROTOCOL_VERSION = '2025-11-25'`, then
`SUPPORTED_PROTOCOL_VERSIONS = [LATEST, '2025-06-18', '2025-03-26', '2024-11-05', '2024-10-07']`.

**Not measured: the revisions of Codex 1.12.0.** The Codex adapter does not
embed this JavaScript package. Negotiation covers the case of a revision Oxyn
does not know (it answers its own, the agent decides), but a **known**
revision whose transport requirements differed — a stream on `GET`, for
example — would not be detected. To check with the next version bump of
either adapter.

### Timeout of an MCP tool call on the agent side — reading of 2026-09-24

An `execute_query` call whose write awaits the user's approval stays suspended
until the decision, five minutes at most (the lifetime of a request,
`oxyn_exec::approval::DEFAULT_TTL`). The question is whether the agent, on its
side, gives up the call earlier. Read in the packages downloaded by `npm pack`
on 2026-09-24, and in Codex's sources at the matching tag — never from memory:

| Agent | What the adapter passes on from Oxyn's MCP server | Timeout applied to a `tools/call` | Source |
|---|---|---|---|
| `@agentclientprotocol/claude-agent-acp` **0.78.0** | `type`, `url`, `headers` — no `timeout` | that of `@anthropic-ai/claude-agent-sdk` **0.3.270**: the server's `timeout`, otherwise the `MCP_TOOL_TIMEOUT` variable, otherwise a default the SDK calls "effectively unbounded" | `dist/acp-agent.js:5864-5872`; `sdk.d.ts:516-519` and the `timeout` field of `McpHttpServerConfig` |
| `@agentclientprotocol/codex-acp` **1.12.0** | `url`, `http_headers` — no `tool_timeout_sec` | Codex's: the server's `tool_timeout_sec`, otherwise `DEFAULT_TOOL_TIMEOUT` = **300 s** | `dist/index.js:28766-28783`; dependency `@openai/codex` `^0.154.0`, which resolves to **0.154.0**; `codex-rs/codex-mcp/src/rmcp_client.rs:103` and `connection_manager.rs:314-317` at tag `rust-v0.154.0` |

**Consequence.** With Claude, the call waits for the decision. With Codex,
Codex's 300 s and the request's five minutes coincide: either deadline wins
within a few milliseconds. In both cases nothing runs, and the card says so —
"Expired", or "Withdrawn: the agent stopped waiting" when Codex hangs up first.
An approval given in the same millisecond runs; Codex has already told its
model that the call expired, and a new attempt on its part waits for the first
to be settled — an agent's calls go one by one
(`crates/oxyn-ai/src/external/mcp/turn.rs`, "one call at a time") — then asks
for approval again, on screen: never a silent replay
([I-13](../CLAUDE.md#i-13)).

**To redo** at every version bump of either adapter or of `@openai/codex`: a
default timeout shorter than five minutes would make the call be given up
before the request's deadline.

### Minimal environment of an ACP agent — measurement of 2026-09-16

Oxyn launches the agent with an **allow-listed** environment
([ADR-0030](adr/0030-outils-oxyn-exposes-a-un-agent-externe.md)). What remains
is knowing what to put in it. Measured by sending a lone `initialize` to each
adapter — no prompt, no database touched — and varying only the environment.
Only the `kind` and `label` of the authentication state are read, never an
environment value.

| Child environment | Claude Agent 0.78.0 | Codex 1.12.0 |
|---|---|---|
| full | `account / Claude Max` | `account / ChatGPT` |
| `PATH`, `HOME`, `LANG`, `TMPDIR` | **`none / Not logged in`** | `account / ChatGPT` |
| + `LOGNAME` | `none / Not logged in` | — |
| + `SHELL` | `none / Not logged in` | — |
| + `SECURITYSESSIONID` | `none / Not logged in` | — |
| + **`USER`** | **`account / Claude Max`** | unchanged |
| `PATH`, `HOME`, `USER` | `account / Claude Max` | — |

**Conclusion: `USER`, and it alone.** No substitute works, and Codex does not
need it — the measurement does not generalize from one adapter to the other.

**The trap, worth more than the value itself.** Without `USER`, `initialize`
**still succeeds**, `authMethods` is `[]` in both cases, and the adapter notes
on `stderr`: `[authStatus] session account carries no identity signal;
keeping`. The refusal only comes at the **prompt**. A check that stops at the
handshake therefore sees nothing, and the user gets "sign in" after asking a
question, while `claude auth status` answers `loggedIn: true` on the same
machine.

This is the cost of an allow-list: **a missing variable does not break the
agent, it silently degrades it**. Hence the rule: each name in the list
carries the measurement that put it there.

### Session modes and options — check of 2026-09-16

An agent can declare its modes twice: through `modes` and through an option of
`configOptions` in the `mode` category. The panel then showed two selectors.

**Schema 1.7.0 does not settle it.** `agent-client-protocol-schema` 1.7.0
declares both fields side by side (`src/v1/agent.rs:877-884`) and mentions
neither replacement nor deprecation. Its only instruction on categories
(`src/v1/agent.rs:2281-2286`): they serve ergonomics and "MUST NOT be
required for correctness".

**The protocol documentation settles it**, read in the site's Markdown source:

| File | Line | Text |
|---|---|---|
| [`protocol/session-config-options.md`](https://agentclientprotocol.com/protocol/session-config-options) | 333 | "Session Config Options supersede the older Session Modes API." |
| same | 340 | "Clients that support config options **SHOULD** use `configOptions` exclusively and ignore `modes`" |
| same | 342 | "Agents **SHOULD** keep both in sync" |
| [`protocol/session-modes.md`](https://agentclientprotocol.com/protocol/session-modes) | 10-11 | "Dedicated session mode methods will be removed in a future version" |

**What Oxyn does with it** (`crates/oxyn-ai/src/external/settings.rs`,
`supersede_modes`): as soon as an option of category `mode` exists, `modes`
and the current mode are no longer retained, displayed, or accepted as a
change. An agent that only declares `modes` keeps its selector: the
instruction targets the transition, not older agents.

### Confinement of ACP adapters — measurement of 2026-09-23

Grounds [ADR-0032](adr/0032-agent-externe-confine-au-lancement.md). Measured
with the pinned adapters (`@agentclientprotocol/claude-agent-acp` 0.78.0,
which embeds `@anthropic-ai/claude-agent-sdk` 0.3.270;
`@agentclientprotocol/codex-acp` 1.12.0, which embeds `@openai/codex`
0.154.0). The measuring client refuses every permission request, like
`permission_for`; the prompt asks the agent to run `touch` on a witness file.

**An agent only asks what its mode makes it ask.**

| Agent, mode | Asks the client? | File created |
|---|---|---|
| Claude Agent, `auto` (initial mode, read from the user's settings) | no | yes |
| Claude Agent, `default` ("Manual") | yes, refusal respected | no |
| Codex, `agent` (initial mode) | no | yes |
| Codex, `read-only`, file in `/tmp` | no | yes |
| Codex, `read-only`, file in the home directory | yes, refusal respected | no |

An agent's mode describes what it decides **without** asking; the default
refusal of `permission_for` only protects what is asked.

**Claude Agent: SDK options go through `_meta`.** In `dist/acp-agent.js`
(0.78.0), `newSession` spreads `params._meta.claudeCode.options` into the SDK
options. Re-read in `sdk.d.ts` (0.3.270):

| Option | Text of `sdk.d.ts` |
|---|---|
| `tools` | `string[] \| { preset }`; `[]` removes all built-in tools |
| `allowedTools` | "List of tool names that are auto-allowed without prompting for permission"; an `mcp__<server>` entry applies to the whole server |
| `strictMcpConfig` | only keeps the MCP servers provided by the caller |
| `settingSources` | the settings sources loaded; `[]` loads none |
| `allowDangerouslySkipPermissions` | required by `bypassPermissions`; the adapter removes that mode from the catalog when `false` |

Measured with `tools: []`, `allowedTools: ["mcp__oxyn"]`,
`strictMcpConfig: true`, `settingSources: []`,
`allowDangerouslySkipPermissions: false`, then `session/set_mode` to
`default`: no tool call on the `touch` prompt, `bypassPermissions` absent from
the modes, and the tool of an `oxyn` MCP server called without a permission
request.

**Codex: `CODEX_CONFIG` and `INITIAL_AGENT_MODE`**, variables documented by the
README of `codex-acp` 1.12.0 ("JSON object merged into the Codex session
config"; "initial mode id: `read-only`, `agent`, or `agent-full-access`").
Keys re-read in the [Codex configuration reference](https://learn.chatgpt.com/docs/config-file/config-reference)
on 2026-09-23:

| Key | Text of the reference |
|---|---|
| `features.shell_tool` | "Enable the default `shell` tool for running commands (stable; on by default)" |
| `features.unified_exec` | "Use the unified PTY-backed exec tool" |
| `features.hooks` | "Enable lifecycle hooks loaded from hooks.json or inline [hooks] config" |
| `features.apps` | "Enable app (connector) integrations (stable; on by default)" |
| `web_search` | `disabled \| cached \| indexed \| live`; "`"disabled"` to remove the tool" |
| `mcp_servers.<id>.url`, `.bearer_token_env_var` | HTTP endpoint, and environment variable carrying the token |
| `mcp_servers.<id>.default_tools_approval_mode` | `auto \| prompt \| writes \| approve` |
| `mcp_servers.<id>.enabled`, `plugins.<id>.enabled` | disabling **one by one**; "No single master key exists" |

Measured with the shell, `unified_exec`, hooks, apps and web search turned
off, in `read-only`:

* the `touch` prompt no longer runs anything; Codex attempts a file edit,
  which asks and is refused;
* an MCP server declared **through ACP** (`session/new`) asks for permission
  at every call, under the `execute` kind and without a title or tool name —
  `default_tools_approval_mode` passed through `CODEX_CONFIG` does not apply
  to it;
* the same server declared **in `CODEX_CONFIG`** with
  `default_tools_approval_mode = "approve"` is called without asking.

**The remaining gap.** Codex still loads the MCP servers and plugins of the
user's `~/.codex/config.toml`; the measurement saw it attempt the tool of a
user plugin. No documented key turns them off as a whole, and the reference
documents no way to ignore that file.

#### The configuration layers Codex loads — re-read on 2026-09-23

Re-read in the source code at tag `rust-v0.154.0` of openai/codex (commit
`6b9826e3aa83b1a5947db50f4332cb9c65f1b340`), which matches `@openai/codex`
0.154.0. The npm registry publishes no more recent 0.154.x at that date,
whereas `codex-acp` 1.12.0 accepts one (`"@openai/codex": "^0.154.0"`). On the
adapter side, re-read in `dist/index.js` of `@agentclientprotocol/codex-acp`
1.12.0, as published on the registry (`gitHead`
`a7afd2ae077d625710194d9701b83595494449de`). Line numbers refer to these two
versions.

**The launch path.** `codex-acp` starts `codex app-server` with no other
argument, with its own environment and its own working directory
(`startCodexConnection`, l. 22098-22106). The CLI starts the app-server with
`LoaderOverrides::default()`: **no profile**, even if `--profile` is passed
([`cli/src/main.rs` l. 1353-1356](https://github.com/openai/codex/blob/rust-v0.154.0/codex-rs/cli/src/main.rs#L1353-L1356)).
At every `session/new`, `codex-acp` sends `thread/start` with two fields
(l. 28596-28602): `cwd`, which takes the ACP `cwd`, and `config`, which
contains `CODEX_CONFIG` plus `projects.<cwd>.trust_level = "trusted"`
(l. 28703-28708). The app-server pours that `config` into the command-line
overrides
([`app-server/src/config_manager.rs` l. 232-243](https://github.com/openai/codex/blob/rust-v0.154.0/codex-rs/app-server/src/config_manager.rs#L232-L243)).
`CODEX_CONFIG` therefore forms the **`SessionFlags`** layer, at the same rank
as `-c`. The app-server's per-thread configuration loader is
`NoopThreadConfigLoader`
([`app-server/src/lib.rs` l. 510](https://github.com/openai/codex/blob/rust-v0.154.0/codex-rs/app-server/src/lib.rs#L510)).

**`CODEX_HOME`**
([`utils/home-dir/src/lib.rs` l. 13-62](https://github.com/openai/codex/blob/rust-v0.154.0/codex-rs/utils/home-dir/src/lib.rs#L13-L62)):

* an **empty** value counts as an absent variable;
* a non-empty value must designate an existing directory. Otherwise, Codex
  stops with an error. The path is canonicalized, and a relative path
  therefore resolves from **Codex's** working directory;
* without `CODEX_HOME`, Codex takes `home_dir()` followed by `.codex`,
  without checking that this directory exists.

**The layers, from weakest to strongest.** The order comes from the
[comment of `load_config_layers_state`](https://github.com/openai/codex/blob/rust-v0.154.0/codex-rs/config/src/loader/mod.rs#L110-L124)
and from the body of that function (l. 132-500).

| Rank | Layer | Source | Can declare `mcp_servers` / `plugins` |
|---|---|---|---|
| 1 | package defaults | `config/defaults.toml`, embedded in the binary | no: 17 lines, neither of these two tables |
| 2 | system | `/etc/codex/config.toml` on Unix ([l. 66](https://github.com/openai/codex/blob/rust-v0.154.0/codex-rs/config/src/loader/mod.rs#L66)) | yes |
| 3 | enterprise-managed cloud | TOML fragments delivered by the server for the connected workspace ([`cloud_config_layers.rs`](https://github.com/openai/codex/blob/rust-v0.154.0/codex-rs/config/src/cloud_config_layers.rs)) | yes |
| 4 | user | `$CODEX_HOME/config.toml` | yes |
| 5 | v2 profile | `$CODEX_HOME/<name>.config.toml`, chosen by `--profile` | not active here, since the app-server ignores the profile. A legacy profile, `[profiles.<name>]`, carries neither of these two tables ([`profile_toml.rs`](https://github.com/openai/codex/blob/rust-v0.154.0/codex-rs/config/src/profile_toml.rs)) |
| 6 | project | every `.codex/config.toml` found between the project root and the thread's `cwd`. The one closest to the `cwd` wins | yes: `mcp_servers` and `plugins` are not in `PROJECT_LOCAL_CONFIG_DENYLIST` ([l. 75-88](https://github.com/openai/codex/blob/rust-v0.154.0/codex-rs/config/src/loader/mod.rs#L75-L88)) |
| 7 | `SessionFlags` | `-c`, `--config`, and the `config` of `thread/start`, that is `CODEX_CONFIG` | this is the layer Oxyn writes |
| 8 | managed, legacy file | `/etc/codex/managed_config.toml` on Unix, **independently of `CODEX_HOME`** ([`layer_io.rs` l. 22, 222-233](https://github.com/openai/codex/blob/rust-v0.154.0/codex-rs/config/src/loader/layer_io.rs#L222-L233)) | yes |
| 9 | MDM-managed (macOS) | managed preference `config_toml_base64` of the `com.openai.codex` domain ([`macos.rs` l. 20-22](https://github.com/openai/codex/blob/rust-v0.154.0/codex-rs/config/src/loader/macos.rs#L20-L22)) | yes |

The public documentation confirms ranks 1 to 7: the "Configuration
precedence" section of
[config-basic](https://learn.chatgpt.com/docs/config-file/config-basic),
re-read on 2026-09-23. It also confirms ranks 8 and 9, in that order, above
`config.toml`: the
[managed-configuration](https://learn.chatgpt.com/docs/enterprise/managed-configuration)
page, re-read the same day. `requirements.toml`, in `/etc/codex/` or in the
`requirements_toml_base64` preference, constrains values but does not form a
configuration layer.

**Merging is recursive, table by table**
([`merge_toml_values`, `merge.rs` l. 58](https://github.com/openai/codex/blob/rust-v0.154.0/codex-rs/config/src/merge.rs#L58)).
`mcp_servers` and `plugins` therefore form the **union** of the entries of all
layers. A stronger layer only replaces the keys it writes. Two consequences:

* `mcp_servers.<id>.enabled = false` at rank 7 turns off a server declared at
  ranks 2 to 6;
* at ranks 8 and 9, that same setting is only overridden if the managed layer
  itself writes `enabled`.

**The project layer**
(`find_project_root`, [l. 1548-1583](https://github.com/openai/codex/blob/rust-v0.154.0/codex-rs/config/src/loader/mod.rs#L1548-L1583);
`discover_project_layers`, [l. 1696-1718](https://github.com/openai/codex/blob/rust-v0.154.0/codex-rs/config/src/loader/mod.rs#L1696-L1718)):

* It starts from the **thread's `cwd`**, not the process's. Oxyn gives both
  the same directory: the private directory `private_directory` creates, both
  working directory of the process and `cwd` of `session/new`.
* The project root is the first ancestor carrying a marker of
  `project_root_markers`. By default, this marker is `.git`: either a file, or
  a directory containing `HEAD`. Without a marker found, the root is the `cwd`
  itself. An empty list disables the walk up.
* Codex reads every `.codex/config.toml` from the root to the `cwd`, skipping
  the one that is `CODEX_HOME`.
* A **non-trusted** layer is loaded but disabled. Trust is looked up first on
  the exact key of the directory in `projects`, then on the project root, then
  on the root of the git repository (l. 1062-1105). The trust `codex-acp` adds
  only targets the `cwd`.

**What a launch by Oxyn sees.** The private directory is new and empty, and
its name is unpredictable. It is created in Oxyn's `TMPDIR`: on macOS,
`/var/folders/…/T/`, with no `.git` above by default. The project root is then
the private directory itself, and no project layer exists. The walk up only
opens if an ancestor of `TMPDIR` carries a `.git`. For example, a `TMPDIR`
placed under a versioned home directory. The user must also have trusted that
ancestor or the repository root.

**What Oxyn does with it.** It reads ranks 2, 4 and 8, and turns off by name
the servers and plugins they declare. It does not present as confined a Codex
whose rank 8 writes `enabled = true`. It reads neither rank 3 nor rank 9:
[ADR-0033](adr/0033-couches-de-configuration-codex.md) decides these three
points.

### What ACP adapters say about an MCP call — re-read on 2026-09-24

Question: how to recognize, in the `tool_call`s an adapter streams, a call to
a tool of Oxyn's MCP server, so as not to draw it a second time next to its
card ([UX-SPEC](UX-SPEC.md#what-the-panel-shows-of-an-external-agent))?
Re-read in the npm registry archives (`npm pack`) on 2026-09-24.

**`@agentclientprotocol/claude-agent-acp` 0.78.0.**

* `dist/tools.js`, `toolInfoFromToolUse`, `default` branch (l. 335-340): a
  tool the adapter does not know, hence any MCP tool, has its name as `title`
  and `"other"` as `kind`.
* `dist/acp-agent.js`, `toolCallNotification` (l. 7186-7210): the `tool_call`
  also carries `name` (unstable field, behind the `unstable_tool_call_name`
  feature of `agent-client-protocol-schema` 1.7.0 and not enabled here). Above
  all it carries `_meta.claudeCode.toolName`, filled by
  `claudeCodeMetaFromToolUse` (l. 7080-7098) with the tool's programmatic
  name. End-of-call updates repeat that `_meta`.
* For an MCP tool, that name is `mcp__<server>__<tool>`, hence
  `mcp__oxyn__describe_schema`.

**`@agentclientprotocol/codex-acp` 1.12.0**, `dist/index.js`:

* `createMcpToolCallUpdate` (l. 23035-23045) and `createExecuteToolCallUpdate`
  (l. 23133-23143): the `tool_call` has `"execute"` as `kind`, not `"other"`.
  Its `title` is `mcp.<server>.<tool>`. Its `rawInput` is
  `{server, tool, arguments}`, and its `_meta` `{is_mcp_tool_call: true}`.
* `completeItemEvent`, case `mcpToolCall` (l. 25123-25130): the final update
  carries the status and the same `rawInput`, **without `_meta`**.
* `createMcpToolProgressEvent` (l. 25289-25299): progress updates carry no
  status.

**What Oxyn does with it.** The signal retained differs per adapter. With
Claude, it is `_meta.claudeCode.toolName`. With Codex, it is
`_meta.is_mcp_tool_call`, with `rawInput.server` and `rawInput.tool`. In both
cases, the name must match exactly a tool the server announced.

The title is not retained. With Claude, a shell command's title is the command
itself, which can be written like an Oxyn tool name. `rawInput` alone is not
retained either: for any other tool, it is the arguments the model writes.

The identifier of a recognized call is kept until its end: the end, with
Codex, does not designate itself.

## Local re-reading of results — 2026-09-10

The Tokio version already resolved, `1.53.1`, stays unchanged. `oxyn-exec`
enables its `rt` feature to hand page decoding over to the blocking pool of
the application's runtime. The official documentation confirms that
[`Handle`](https://docs.rs/tokio/1.53.1/tokio/runtime/struct.Handle.html) is
available under `rt`, that `try_current` returns an error in the absence of a
runtime and that `spawn_blocking` uses an executor dedicated to blocking
operations. Consulted on 2026-09-10; no new dependency and no version bump.
Budget sharing and the limits of this re-reading are decided in
[ADR-0012](adr/0012-lecture-pages-resultats.md).

## GPUI shutdown and preferences — 2026-09-10

[`App::shutdown`](https://docs.rs/gpui/0.2.2/gpui/struct.App.html#method.shutdown)
of GPUI `0.2.2` grants 100 ms to the `on_app_quit` handlers, according to its
official documentation consulted on 2026-09-10 and the code resolved locally.
Waiting for the save of the last window is therefore placed before the call to
`quit`, in addition to the hook. This does not prove every native shutdown
path; their acceptance testing stays tracked in IMPLEMENTATION-PLAN. No
dependency version changed.

## Cancellation of local searches — 2026-09-10

`oxyn-store` enables `hooks` on the version of `rusqlite` already resolved,
`0.37.0`. No version or lock entry changes. The installed source code
`rusqlite-0.37.0/src/hooks/mod.rs` exposes `Connection::progress_handler` and
its deactivation through `progress_handler(0, None::<fn() -> bool>)`; its
manifest confirms the `hooks` feature. Local check of 2026-09-10; consulting
docs.rs failed in the browsing tool.

The handler is installed under the lock of the local connection and removed by
a scope guard. It observes the token of the only active operation every 1,000
steps of the SQLite virtual machine. Waiting for the lock and SQLite's busy
timeout are not interrupted by this handler; a task cancelled while waiting
does not start its operation after acquiring the lock. A write already
committed keeps its success result.

The global `verifier_versions.py` check of 2026-09-10 also reports Redis
`1.7.0` on the registry against `1.6.0` in the historical reading of
2026-09-05. This gap concerns a later driver lead, not the rusqlite dependency
changed here; no version bump is made in this batch.


## In-memory SQLite sessions — 2026-09-10

The [SQLite documentation](https://www.sqlite.org/inmemorydb.html) distinguishes
private databases opened under `:memory:` from named in-memory databases opened
with `mode=memory&cache=shared`. The latter are shared by the connections of a
single process using the same name and disappear when the last connection
closes. Source consulted on 2026-09-10.

The driver now uses an internal name derived from the configuration identity
so that the sessions of the same profile find their in-memory database again.
Two profiles stay isolated. This name does not appear in diagnostics. The
read-only restriction is applied through `query_only`, read back after
opening, then kept within the limits of each query; a caller requesting
writable limits cannot lift it. The contract tests check shared reading, write
refusal, survival of a neighboring session and a new empty database after the
last session closes. No dependency or version changed.

### Constraint introspection — check of 2026-09-10

`pg_constraint` provides `contype`, `conkey` and the names; the columns of
composite keys are read back in their order. `pg_get_constraintdef` produces a
reconstruction by the engine, not the text originally typed.
Sources: [PostgreSQL constraint catalog](https://www.postgresql.org/docs/current/catalog-pg-constraint.html)
and [information functions](https://www.postgresql.org/docs/current/functions-info.html).
NOT NULL attributes without a matching entry are read from
`pg_attribute.attnotnull`, without inventing a name for them. The limits of
1024 entries and 16 KiB per definition are product limits, with an explicit
error.

The compatibility test for old caches only adds
`serde_json.workspace = true` to the test dependencies of `oxyn-catalog`.
The workspace version **1.0.151** is kept, checked with
`cargo info serde_json@1.0.151`; [registry](https://crates.io/crates/serde_json/1.0.151).
No new version is introduced.

### SQLite constraints — check of 2026-09-10

PK, UNIQUE, CHECK, NOT NULL and REFERENCES constraints are declared in the SQL
stored by SQLite. PRAGMAs are not enough to restore their names and their
CHECK clauses. The driver's extraction keeps the original slices of
`sqlite_schema.sql`; it does not rebuild a CREATE TABLE from the AST.
Sources: [SQLite CREATE TABLE](https://www.sqlite.org/lang_createtable.html)
and [schema table](https://www.sqlite.org/schematab.html).
Adjacent table constraints without a comma, quoted names, comments and
`ON CONFLICT` clauses are checked against the SQLite engine embedded by the
driver's tests. The bound of 1 MiB of source SQL, the 1024 constraints and the
16 KiB per clause are product limits; any overflow is reported. No new
dependency is added.

The status of the Constraints grid (`229:7998`) relies on
`pg_constraint.convalidated`, checked in the PostgreSQL documentation cited
above. A `NOT VALID` CHECK and its move to `VALIDATE CONSTRAINT` are covered by
a test on a throwaway database. SQLite keeps an unknown status, including when
data not respecting a CHECK was inserted with `ignore_check_constraints`
enabled; a test prevents presenting the mere presence of the clause as proof of
validation.

### Incoming relations — check of 2026-09-10

SQLite PRAGMA functions can be used in SELECT and accept the schema as their
last argument: [documentation](https://www.sqlite.org/pragma.html#pragma_functions).
References without a column list use the target's primary key; their
comparison uses the affinity and collation of the parent columns:
[SQLite foreign keys](https://www.sqlite.org/foreignkeys.html).
The driver reads native metadata through `Connection::column_metadata`, an API
checked in the installed rusqlite source, without adding a dependency.

PostgreSQL exposes the target in `pg_constraint.confrelid`, the column lists in
`conkey`/`confkey` and the indexes in `pg_index`.
INCLUDE columns are excluded from the uniqueness test by using `indnkeyatts`;
invalid, partial, expression or non-established-comparison indexes do not
allow certifying a one-to-one relation.
Sources: [pg_constraint](https://www.postgresql.org/docs/current/catalog-pg-constraint.html)
and [pg_index](https://www.postgresql.org/docs/current/catalog-pg-index.html).
The limits of 1024 keys, 128 columns per key, 16 KiB per text and 16 MiB of
SQLite texts scanned are product limits, not limits attributed to the DBMSs.

### DDL definitions — check of 2026-09-10

SQLite keeps the statements in
[sqlite_schema](https://www.sqlite.org/schematab.html). The reader takes
object, indexes and triggers in a single cursor; qualification only changes
their declaration names. A test recreates them in a separate attached space
and checks that the trigger works and that no object was created in `main`.

PostgreSQL exposes reconstruction functions (`pg_get_viewdef`,
`pg_get_indexdef`, `pg_get_triggerdef`, `pg_get_ruledef`,
`pg_get_constraintdef`) in its [information functions](https://www.postgresql.org/docs/current/functions-info.html).
Sequence parameters come from
[pg_sequence](https://www.postgresql.org/docs/current/catalog-pg-sequence.html).
The form of generated/identity columns and of partitioned tables follows
[CREATE TABLE](https://www.postgresql.org/docs/current/sql-createtable.html).
The reader uses `attgenerated`, present in
[pg_attribute of PostgreSQL 12](https://www.postgresql.org/docs/12/catalog-pg-attribute.html):
`OBJECT_DEFINITION` is only announced from that recognized version on, and not
for Redshift. Older or unreadable versions do not get this capability by
assumption.

RLS policies are read back from
[pg_policy](https://www.postgresql.org/docs/17/catalog-pg-policy.html), with the
role names of the public view `pg_roles` (OID 0 means PUBLIC). The
[ENABLE/FORCE ROW LEVEL SECURITY](https://www.postgresql.org/docs/17/sql-altertable.html)
states are rebuilt. The tests on PostgreSQL 17.11 check that they are kept
after recreation, as well as identity/serial, generated columns, indexes,
triggers, views, sequences and partitioned roots. The scripts are only applied
by these explicitly isolated fixtures, through sqlx's `AssertSqlSafe` after
audit. No product path automatically applies the displayed definition.

### PostgreSQL partitions and current statement — check of 2026-09-10

Creating a child uses
[PARTITION OF](https://www.postgresql.org/docs/17/sql-createtable.html), the
direct link of [pg_inherits](https://www.postgresql.org/docs/17/catalog-pg-inherits.html)
and the bound of [pg_class](https://www.postgresql.org/docs/17/catalog-pg-class.html).
The DDL keeps local options and lets PostgreSQL clone the inherited elements.
The recreation tests check parent, bound, default and sub-partition.

The current statement is resolved in `oxyn-query`, without native dependency
or I/O. The scanner keeps SQLite/PG bodies; invalid UTF-8 positions,
incomplete texts and SQLite identifiers making the boundary ambiguous produce
a request for an explicit selection. This resolution does not replace the
bus classifier and adds no new dependency version.

The compatibility declared from PostgreSQL 12 on takes into account the
absence of `inhdetachpending` in [pg_inherits 12](https://www.postgresql.org/docs/12/catalog-pg-inherits.html)
and of `tgparentid` in [pg_trigger 12](https://www.postgresql.org/docs/12/catalog-pg-trigger.html).
These fields are read through JSONB, without a direct SQL reference that would
break reading ordinary tables. Without native provenance of triggers, the
definition of a child carrying some is refused rather than inferred from
similar names or expressions. The real tests of this batch use PostgreSQL
17.11; a PostgreSQL 12 server was not run.

## End of a `--` comment — check of 2026-09-16

The splitter of `oxyn-query` decides which text is a statement, and the
classifier infers from it that it writes. It must end a line comment **where
the server ends it**. Neither choice is safe by default.

- **Ending too late** hides a statement the server runs.
- **Ending too early** reads as code a `/*` or an apostrophe the server reads
  as comment. The following statement then disappears into a fake block
  comment.

| Engine | End of `--` | Source |
|---|---|---|
| PostgreSQL, server | `\n` **or** `\r` | [`scan.l` at tag `REL_17_6`](https://github.com/postgres/postgres/blob/REL_17_6/src/backend/parser/scan.l#L224-L227), l. 224-227: `newline [\n\r]`, `comment ("--"{non_newline}*)`. Same definition on `master` at commit `a4f18fd8f280`, l. 206-209 |
| PostgreSQL, `psql` | `\n` or `\r` | [`psqlscan.l` at tag `REL_17_6`](https://github.com/postgres/postgres/blob/REL_17_6/src/fe_utils/psqlscan.l#L160-L163), l. 160-163 |
| SQLite | `\n` or NUL, **not** `\r` | [`tokenize.c`](https://github.com/sqlite/sqlite/blob/version-3.50.2/src/tokenize.c), `sqlite3GetToken`, `case CC_MINUS`: `for(i=2; (c=z[i])!=0 && c!='\n'; i++){}`. Identical in the **3.50.2** amalgamation that `libsqlite3-sys` `0.35.0` embeds (`sqlite3.c`, l. 181599) |
| `sqlparser` `0.62.0` | `\n`; also `\r` for `PostgreSqlDialect` only | `src/tokenizer.rs`, `tokenize_single_line_comment`, l. 2039-2044 |

`/* */` comments give `\r` no role. PostgreSQL nests them, SQLite does not.

**Checked at runtime.** A throwaway PostgreSQL 17.11 was queried with the
simple protocol and with the extended Parse/Bind/Execute/Sync protocol, the one
`sqlx` uses on `prepare`.

- `-- x\rDROP TABLE audit` drops the table in both protocols.
- `SELECT 1; -- x\rDROP TABLE audit` drops it with the simple protocol. With
  the extended protocol, the server answers `42601 cannot insert multiple
  commands into a prepared statement` and runs nothing.
- An empty leading `;` does not count as a command:
  `; -- x\rDROP TABLE audit` goes through with the extended protocol.

SQLite 3.53.3 was queried through `executescript`: the text following an
isolated `\r` stays a comment.

**What Oxyn does with it.** `LineCommentEnd`, in
`crates/oxyn-query/src/split.rs`, sets the end of a comment per dialect:

- `Postgres`: `\r` or `\n`;
- `Sqlite`: `\n`;
- any dialect **not checked** here, Redshift included (its lexer is closed): a
  line comment containing an isolated `\r` followed by text makes its
  statement unreadable, hence `Unknown`.

Not checked:

- MySQL, SQL Server, ClickHouse, DuckDB, Snowflake, BigQuery, Oracle;
- the handling of a NUL in PostgreSQL's Parse message.

## Anthropic provider — check of 2026-09-16

Read for the implementation of `crates/oxyn-llm/src/anthropic/`. **The
documentation changed domain**: `docs.anthropic.com` and `docs.claude.com`
answer `301`/`302` towards `platform.claude.com/docs/en/…`. The URLs below are
those that answer directly.

### Transport

| Fact | Value | Source | Checked on |
|---|---|---|---|
| Generation | `POST /v1/messages` | [Messages API](https://platform.claude.com/docs/en/api/messages) | 2026-09-16 |
| Token counting | `POST /v1/messages/count_tokens`, response `{"input_tokens": N}` | [Token counting](https://platform.claude.com/docs/en/build-with-claude/token-counting) | 2026-09-16 |
| Model list | `GET /v1/models`, `limit` from 1 to 1000 (default 20), cursors `after_id`/`before_id` | [List Models](https://platform.claude.com/docs/en/api/models/list) | 2026-09-16 |
| Required headers | `x-api-key`, `anthropic-version`, `content-type: application/json` | [Messages API](https://platform.claude.com/docs/en/api/messages) | 2026-09-16 |
| Value of `anthropic-version` | `2023-06-01` | same, and cURL examples of every page consulted | 2026-09-16 |
| Required field | `max_tokens` (with `model` and `messages`) | same | 2026-09-16 |
| Maximum request size | 32 MB on Messages and on counting | [Errors § request size limits](https://platform.claude.com/docs/en/api/errors) | 2026-09-16 |

> **`temperature`, `top_p` and `top_k` are documented as deprecated**, and
> recent models only accept `1.0` (respectively `≥ 0.99`). Oxyn only sends
> them if the caller explicitly set them — the behavior was already that, and
> this note says why it must not be "fixed" by setting a default.

### SSE stream

Eight event types, each carrying its name in `event:` **and** a `type` field in
its payload. The decoder reads the field, not the name: the two are redundant
and the payload is what a proxy alters least.

| Event | What it carries |
|---|---|
| `message_start` | the `message` object, empty `content`, input `usage` already filled |
| `content_block_start` | `index` and `content_block` (with its `type`) |
| `content_block_delta` | `index` and `delta` |
| `content_block_stop` | `index` |
| `message_delta` | `delta.stop_reason`, `delta.stop_sequence`, and **cumulative** `usage` |
| `message_stop` | nothing |
| `ping` | nothing — keep-alive, in any number |
| `error` | `error.type` and `error.message`, after a `200` |

Four `delta` types, all checked against the examples of the
[Streaming](https://platform.claude.com/docs/en/build-with-claude/streaming)
page: `text_delta` (`text`), `input_json_delta` (`partial_json`),
`thinking_delta` (`thinking`), `signature_delta` (`signature`). The
`signature_delta` arrives just before the `content_block_stop` of the thinking
block.

Content block types encountered: `text`, `tool_use`, `thinking`,
`redacted_thinking` (field `data`), `server_tool_use`,
`web_search_tool_result`. Oxyn offers no server-side tool; the last two are
tracked without being used.

> **There is no end sentinel.** Unlike OpenAI-compatible protocols, no
> `[DONE]`: the end is `message_stop`. A stream closing without
> `message_stop` — cleanly or not — is not an end: Oxyn classifies it as
> `StopReason::Interrupted` and discards the blocks without
> `content_block_stop`.

### Stop reasons

Seven values, [Handling stop reasons](https://platform.claude.com/docs/en/build-with-claude/handling-stop-reasons),
checked on 2026-09-16: `end_turn`, `max_tokens`, `stop_sequence`, `tool_use`,
`pause_turn`, `refusal`, `model_context_window_exceeded`. The versioning page
announces that this list may grow: an unknown value is **kept**
(`StopReason::Other`) and never folded into `end_turn`.

### Errors and retry

| Status | `error.type` | Family retained |
|---|---|---|
| 400 | `invalid_request_error` | permanent |
| 401 | `authentication_error` | permanent |
| 402 | `billing_error` | permanent |
| 403 | `permission_error` | permanent |
| 404 | `not_found_error` | permanent |
| 409 | `conflict_error` | permanent |
| 413 | `request_too_large` | permanent |
| 429 | `rate_limit_error` | **transient** |
| 500 | `api_error` | **transient** |
| 504 | `timeout_error` | **ambiguous** |
| 529 | `overloaded_error` | **transient** |

Source: [Claude API errors](https://platform.claude.com/docs/en/api/errors).

#### Replaying a `500`, `502` or `504` — reading of 2026-09-17

| Status | Anthropic ([Claude API errors](https://platform.claude.com/docs/en/api/errors)) | OpenAI ([Error codes](https://developers.openai.com/api/docs/guides/error-codes)) | Family retained |
|---|---|---|---|
| `500` | `api_error`: "Retry the request with exponential backoff" | "Retry your request after a brief wait and contact us if the issue persists" | transient — **sourced** at both |
| `502` | not documented | not documented | transient — **not verified**: earlier classification kept for lack of a source |
| `504` | `timeout_error`: "The request timed out **while processing**" | not documented | **ambiguous** |

A `504` is not an overload: processing had started, and the response may have
been produced — and billed — without arriving (I-13). A gateway placed in
front of a compatible provider likewise answers `504` after forwarding the
request. It is therefore ambiguous, like `LlmError::ResponseTimeout` and
`LlmError::ConnectionLost`, and maps to `OxynError::OutcomeUnknown`.

Neither page says whether a failed request was billed. The Anthropic page adds
that its SDKs replay "5xx server errors" on their own: that is an SDK choice,
not a guarantee of no effect, and Oxyn never replays on its own.
Error body: `{"type":"error","error":{"type":…,"message":…},"request_id":…}`.

> **`529` got the code fixed.** It belongs to no HTTP standard, and the table
> of `LlmError::class` put it in "rest of the `5xx`", hence permanent: the
> interface would have offered "reconfigure" for a passing overload. It is now
> transient.

The `retry-after` header is returned on a rate-limiting `429`, **in seconds**
([Rate limits § response headers](https://platform.claude.com/docs/en/api/rate-limits)).
Two documented exceptions where it is absent: the monthly spend cap `429`,
recognizable by `error.details.error_code = enforced_spend_limit_reached`, and
for which replaying fails until the next month. Oxyn never replays on its own
(I-13): this header is therefore not read yet, and noting it here keeps anyone
from believing it is handled.

### Reasoning: two distinct settings

This is the point that gets missed, because the two names look alike and do
not do the same thing.

| Setting | Where it lives | Values | Source |
|---|---|---|---|
| Effort | `output_config.effort`, top level of the body | `low`, `medium`, `high`, `xhigh`, `max` | [Effort](https://platform.claude.com/docs/en/build-with-claude/effort) |
| Thinking mode | `thinking.type` | `adaptive`, `enabled` (with `budget_tokens`), `disabled` | [Thinking](https://platform.claude.com/docs/en/build-with-claude/thinking) |
| Rendering of the reasoning | `thinking.display` | `summarized`, `omitted` (default on several models), `updates` (beta) | same |

Two verified traps, and they dictate the code:

1. **`thinking.type: "adaptive"` is refused by earlier models** with a `400`
   (`adaptive thinking is not supported on this model`), and
   `thinking.type: "enabled"` is refused by recent models, which point to the
   `adaptive` + `output_config.effort` pair. No mode is therefore universal:
   Oxyn sends `thinking` **only** if the caller asks for a budget, and settles
   for `output_config.effort` otherwise.
2. **`display` defaults to `omitted`** on most models: the thinking block then
   comes back with an empty `thinking` but **a signature**. It must still be
   kept and sent back, otherwise the next turn fails.

Thinking blocks are sent back **unchanged**: the API checks their signature,
and a modification produces a `400` whose message is
`` `thinking` or `redacted_thinking` blocks in the latest assistant message
cannot be modified ``. `redacted_thinking` blocks count, including those whose
`thinking` field is empty.

#### Which models accept an effort, and how Oxyn knows

**The code contains no model list.** `ModelInfo::reasoning_efforts` is filled
at runtime, level by level, from
`capabilities.effort.{low,medium,high,xhigh,max}.supported` of the response of
`GET /v1/models` — a single caller, `crates/oxyn-llm/src/anthropic/wire.rs`.
This is what makes it impossible for any value to go stale: the provider
declares, Oxyn relays, and an **empty** list means "not declared", never
"none".

The list below is a **cross-reference** for knowing what to expect — it feeds
no code path, and it will go stale. Read on 2026-09-16 on the
[Effort](https://platform.claude.com/docs/en/build-with-claude/effort) page,
section "Supported models":

> `claude-fable-5-1`, `claude-mythos-5-1`, `claude-fable-5`, `claude-mythos-5`,
> `claude-mythos-preview`, `claude-opus-5`, `claude-opus-4-8`,
> `claude-opus-4-7`, `claude-opus-4-6`, `claude-opus-4-5-20251101`,
> `claude-sonnet-5`, `claude-sonnet-4-6`.

Two caveats from the same page, and they matter for the interface: **`xhigh`
is not offered everywhere** ("Not every model that supports `max` supports
`xhigh`"), and the default is `high` — "setting `effort` to `"high"` produces
exactly the same behavior as omitting the `effort` parameter". A selector
showing the five levels for every model would therefore produce an `xhigh`
button that fails on part of the catalog; this is exactly why the list comes
from the API and not from here.

The list endpoint declares these capabilities per model:
`capabilities.thinking.supported`,
`capabilities.thinking.types.{adaptive,enabled}.supported`, and
`capabilities.effort.{supported,low,medium,high,xhigh,max}.supported` — the
documentation calls the latter "Effort (reasoning\_effort) support". This is
what feeds `ModelInfo::supports_reasoning` and `ModelInfo::reasoning_efforts`.

### Prompt caching

| Fact | Value | Source |
|---|---|---|
| Marker | `"cache_control": {"type": "ephemeral"}` | [Prompt caching](https://platform.claude.com/docs/en/build-with-claude/prompt-caching) |
| Duration | `ttl` `"5m"` (default) or `"1h"`, **without** a beta header | same |
| Locations | `system` blocks, content blocks of `messages`, **last** tool of `tools` | same |
| Maximum number of markers | **4**; a fifth makes the request fail | same |
| Minimum cached length | 512 to 4096 tokens depending on the model; below that, no cache and **no error** | same |
| Consumption | `cache_creation_input_tokens` (written), `cache_read_input_tokens` (read) | same |

The input total is `input_tokens + cache_creation_input_tokens +
cache_read_input_tokens`: `input_tokens` only counts what follows the last
marker. This is why `ChatEvent::Usage` carries them separately rather than
adding them up.

Oxyn sets at most four markers and stops before the limit rather than letting
a `400` arrive for a setting the user is not aware of having set.

### Tools

`input_schema` and not `parameters`. A tool result is a
`{"type":"tool_result","tool_use_id":…,"content":…}` block in a message with
role `user`, with `is_error: true` when execution failed. Fine-grained
streaming is enabled **per tool** with `eager_input_streaming: true`, replacing
the beta header `fine-grained-tool-streaming-2025-05-14`
([Fine-grained tool streaming](https://platform.claude.com/docs/en/agents-and-tools/tool-use/fine-grained-tool-streaming));
the documentation then warns that the accumulated JSON **may be invalid**.
Oxyn does not enable it, but keeps the parse guarded: a `max_tokens` reached in
the middle of a parameter produces the same truncated JSON, with no option at
all.

### Prices — deliberately absent

`GET /v1/models` publishes **no** price, and none of the pages consulted gives
any in a machine-readable form. `ModelInfo::cost` therefore stays `None` for
this provider. Writing a hard-coded price grid would be exactly the plausible
and wrong value that I-12 forbids; OpenRouter, which publishes its prices in
its response, remains the only provider whose cost is filled in (see
[OpenRouter provider](#openrouter-provider--check-of-2026-09-24)).

### On the OpenAI side, for parity

| Fact | Value | Source | Checked on |
|---|---|---|---|
| Reasoning effort | `reasoning_effort` at the top level (Chat Completions); `reasoning.effort` on the Responses API | [Reasoning](https://developers.openai.com/api/docs/guides/reasoning) | 2026-09-16 |
| Values | model-dependent, among `none`, `minimal`, `low`, `medium`, `high`, `xhigh`, `max` | same | 2026-09-16 |
| Cached tokens | `usage.prompt_tokens_details.cached_tokens` | [Chat object](https://developers.openai.com/api/docs/api-reference/chat/object) | 2026-09-16 |
| Reasoning tokens | `usage.completion_tokens_details.reasoning_tokens` | same | 2026-09-16 |
| Refusal | `delta.refusal`, a field distinct from `delta.content` | same | 2026-09-16 |
| Stop reasons | `stop`, `length`, `tool_calls`, `content_filter`, `function_call` (deprecated) | same | 2026-09-16 |
| End of stream | `finish_reason` filled on the last content chunk; then, with `stream_options.include_usage`, a chunk with empty `choices` carrying `usage`; then `data: [DONE]`. "If the stream is interrupted, you may not receive the final usage chunk" | [Create chat completion](https://developers.openai.com/api/docs/api-reference/chat/create) | 2026-09-16 |

`ReasoningEffort` only carries the five levels common to both providers.
`none` and `minimal` are OpenAI-specific; the enumeration is
`#[non_exhaustive]` and `ReasoningEffort::parse` returns `None` on them rather
than folding them into `low`, which would change the user's request.

### End of stream on local OpenAI-compatible servers

Oxyn classifies as `StopReason::Interrupted` a stream closed with neither
`finish_reason` nor `[DONE]`. The risk measured: a local server that never sent
`finish_reason` would see all its responses marked interrupted. Read **in the
sources**, on 2026-09-16, at the named revisions.

| Server | Revision | Final `finish_reason` | `[DONE]` | Context limit | Error mid-stream |
|---|---|---|---|---|---|
| Ollama | [`a43fad18`](https://github.com/ollama/ollama/tree/a43fad18b088095de20fbd7a8f0de50824cf5d27) (`main`, 2026-09-15) | always, on a dedicated frame: `FinishChunk` (`openai/openai.go`) is `DoneReason`, `stop` by default, `tool_calls` if a call was emitted | yes, after the final frame and the possible `usage` frame (`ChatWriter.writeResponse`, `middleware/openai.go`) | `length`: the engine is `llama-server`, whose `limit` stop becomes `DoneReasonLength` (`llm/llama_server.go`); an open generation is bounded to several context windows (`boundedNumPredict`) | **neither `finish_reason` nor `[DONE]`**, and the message is lost: `streamResponse` (`server/routes.go`) writes `{"error":…}` after a `200`, which `ChatWriter.Write` re-reads as an empty response. Oxyn: `Interrupted` |
| llama.cpp | [`60199339`](https://github.com/ggml-org/llama.cpp/tree/60199339bcff9092dd7273d371b52308c85a92de) (`master`, 2026-09-16) | always, on a dedicated frame: `to_json_oaicompat_chat_stream` (`tools/server/server-task.cpp`) is `stop` or `tool_calls` on end of model or stop word, `length` otherwise | yes, when no more results arrive (`tools/server/server-context.cpp`) | `length`: without context shifting, `STOP_TYPE_LIMIT` when the window is full; with it, generation continues by shifting | `data: {"error":…}`, then closing **without** `[DONE]`. Oxyn: `ProviderError` |
| LM Studio | — | **not measured**: closed software | not measured | not measured | not measured |

Conclusion: the rule holds for both open servers. A stop at the context limit
is a `MaxTokens` there, not a cut. llama.cpp also sends `:` SSE keep-alive
comments, which the decoder ignores. The facts
> above come from `developers.openai.com`, which serves the same
> documentation — it is also the domain already used above in this file for
> the page of `gpt-6-astra`.

## Gemini provider — check of 2026-09-24

| Fact | Value | Source | Checked on |
|---|---|---|---|
| Host | `https://generativelanguage.googleapis.com` | [Generate content](https://ai.google.dev/api/generate-content) | 2026-09-24 |
| Path version | `v1beta` | same | 2026-09-24 |
| Stream path | `models/{model}:streamGenerateContent` | same | 2026-09-24 |
| Query parameter | `alt=sse` — without it the API returns a whole JSON array rather than a stream | same | 2026-09-24 |
| Key header | `x-goog-api-key` | [API versions](https://ai.google.dev/gemini-api/docs/api-versions) | 2026-09-24 |

`v1beta` rather than `v1`: the reference for `generateContent` and
`streamGenerateContent` only documents the path under `/v1beta/…`. The API
versions page confirms that `v1` is the stable version while `v1beta` "carries
the recent features" and remains the default of the official SDKs.
`GeminiProvider::with_api_version` allows switching to `v1` the day Google
migrates `streamGenerateContent` there, without touching `GEMINI_BASE_URL`.

The key goes through the `x-goog-api-key` header — shown in the `curl`
examples of the API versions page — and not through the `?key=…` parameter
that the `generateContent` reference also uses in its own examples: a header
never ends up in a logged URL, a query parameter does
([I-03](../CLAUDE.md#i-03), [SECURITY](SECURITY.md#secrets)).

[ARCHITECTURE §7.5](ARCHITECTURE.md#75-provider-abstraction)
establishes the real state of the provider: `GeminiProvider` builds this
request then refuses the call (`LlmError::NotImplemented`) before any
`.send()`. The exact schema of the response stream and the list of available
models remain unverified; no value is written for them here.

## OpenRouter provider — check of 2026-09-24

| Fact | Value | Source | Checked on |
|---|---|---|---|
| Price currency | `USD` | [Models](https://openrouter.ai/docs/guides/overview/models) | 2026-09-24 |
| Unit | price per token, returned as a string; `WirePricing::into_cost` converts it into a price per million | same | 2026-09-24 |
| `prompt` field | cost per input token | same | 2026-09-24 |
| `completion` field | cost per output token | same | 2026-09-24 |

The API response never carries the currency — only this documentation page
gives it. This is why `OPENROUTER_CURRENCY` is a constant injected on Oxyn's
side rather than a field deserialized from the wire, and this row is its
source. The page consulted says nothing about a `-1` value: the reading of
`parse_price` (`crates/oxyn-llm/src/openai_compatible/wire.rs`) — which
rejects any negative price, `-1` included — therefore remains, on this precise
point, unconfirmed by the provider's documentation.

## Benchmarks — check of 2026-09-10

`criterion` is at **`0.8.2`**, read at the registry by `cargo search
criterion` on 2026-09-10. It is adopted at that date, as a workspace
`[dev-dependencies]`, and only enters the graph through the `[[bench]]`
targets of `oxyn-query` and `oxyn-driver-sqlite`.

Features declared by the published version, read by
`cargo add --dry-run --dev criterion`: `cargo_bench_support`, `plotters` and
`rayon` are enabled by default; `async`, `async_futures`, `async_smol`,
`async_tokio`, `csv_output`, `html_reports`, `real_blackbox` and `stable` are
not. None is enabled beyond the defaults: benchmarks that need an executor
build their own `tokio::runtime` and call `block_on`, which avoids adding
`async_tokio` — the crate has no business knowing our executor.
`real_blackbox` is useless since `std::hint::black_box` is stable, and that is
the one the benchmarks use.

Two usage points checked at runtime, and not inferred from the documentation:

- a `criterion` executable launched **without** the `--bench` argument goes
  into test mode and prints no measurement. It reports nothing other than
  `Success`: a campaign launched that way looks like it ran;
- `BenchmarkGroup::sample_size` **overrides** the command-line
  `--sample-size` argument. A value written in the code therefore cannot be
  tuned at invocation.

The measurement campaign itself, its conditions and its results are in
[PERFORMANCE](PERFORMANCE.md#measurement-campaign-of-2026-09-10): this file only
carries the external facts.

## Menus, windows and destructive DDL — check of 2026-09-25

Facts that
[ADR-0041](adr/0041-registre-d-actions-menus-et-raccourcis.md),
[ADR-0042](adr/0042-revue-sur-place-des-operations-destructrices.md) and
[ADR-0043](adr/0043-multi-fenetre.md) rely on. On the Tauri side, read in the
source of the `Cargo.lock` versions unpacked in the local registry: `tauri`
2.11.5, `tauri-utils` 2.9.3, `tauri-runtime-wry` 2.11.4, `wry` 0.55.1, `muda`
0.19.3; on the front-end side, `@codemirror/commands` 6.11.0, a transitive
dependency of `@uiw/react-codemirror`. What tao does with `terminate:`
(predefined Quit, Dock, logging out) is in the table of
[Tauri interface and front end](#tauri-interface-and-front-end), and the behavior of
the message dialog without a parent window under the list of ADR-0037:
neither is repeated here.

| Subject | Verified fact | Source |
|---|---|---|
| Default macOS menu | Without a provided menu, `Builder::build` sets `Menu::default` (application, File, Edit, View, Window, Help) as long as `enable_macos_default_menu` is `true`, the default; nothing on Windows and Linux | `tauri` `src/app.rs`, `src/menu/menu.rs` |
| System roles | The Edit entries of `muda` are AppKit selectors (`copy:`, `paste:`, `cut:`, `selectAll:`, `undo:`, `redo:`); Quit is `terminate:`, Close Window `performClose:` | `muda` `src/platform_impl/macos/mod.rs` |
| Predefined accelerators | On macOS, `Close Window` carries `⌘W`, `Quit` `⌘Q`, `Hide` `⌘H`, `Hide Others` `⌘⌥H`; elsewhere, `Close Window` carries `Alt+F4` | `muda` 0.19.3 `src/items/predefined.rs` |
| Menu item | `set_text`, `set_enabled`, `set_accelerator`; no tooltip; a displayed shortcut is a bound shortcut | `muda` 0.19.3 |
| Permissions | `event` and `menu` are core permissions: `listen` on the JS side requires one; the application's commands and `Channel`s are not subject to them | `tauri` 2.11.5; description of `capabilities/main.json` |
| Title bar | `titleBarStyle` only applies to macOS | `tauri-utils` `src/config.rs` |
| Zoom and accelerators | `zoomHotkeysEnabled` is `false`; `back_forward_navigation_gestures` is `false`; `with_browser_accelerator_keys` of `wry` (true by default under WebView2) is not exposed by `tauri-runtime-wry`; `allowLinkPreview` is `true` on macOS | `tauri-utils` `src/config.rs`, `wry`, `tauri-runtime-wry` |
| File drop | `dragDropEnabled` is `true`; "Disabling it is required to use HTML5 drag and drop on the frontend on Windows" | `tauri-utils` `src/config.rs` |
| Capability | The `windows` field accepts a glob pattern | `tauri-utils` `src/acl/capability.rs` |
| Caller identity | A command can receive the `Webview` or `WebviewWindow` that invokes it, provided by the runtime | `tauri` `src/webview/mod.rs`, `src/webview/webview_window.rs` |
| `Channel` | Delivers to the webview that created it, and to it alone | `tauri` `src/ipc/channel.rs` |
| Menu events | The listener is global to the application, whatever the window | `tauri`, doc of `on_menu_event` |
| Window creation | "deadlocks when used in a synchronous command or event handlers" on Windows | `tauri`, doc of `WebviewWindowBuilder::new` |
| Last window | Its destruction emits `RunEvent::ExitRequested { code: None }`, preventable; `RunEvent::Reopen` only exists on macOS | `tauri-runtime-wry` `src/lib.rs`; `tauri` `src/app.rs` |
| Dialog with parent window | On macOS, `rfd` presents a message dialog without a parent through `utils::sync_pop_dialog` or `async_pop_dialog` — the `CFUserNotificationDisplayAlert` alert of ADR-0037 —, and with a parent through an `NSAlert` as a sheet (`beginSheetModalForWindow`) | `rfd` 0.16.0 `src/backend/macos/message_dialog.rs`, `show` and `show_async` |
| Window template | A window declared `"create": false` serves as a template for `WebviewWindowBuilder::from_config` | `tauri-utils`, `WindowConfig::create` |
| Template label | `WindowBuilder::from_config` takes the label from `config.label`: a copy of the template whose `label` is changed builds a window under that label | `tauri` 2.11.5 `src/window/mod.rs`, `from_config` |
| Emission | `Emitter` carries six emission methods: `emit`, `emit_str`, `emit_to`, `emit_str_to`, `emit_filter`, `emit_str_filter` — forbidden by `clippy.toml` (ADR-0043) | `tauri` 2.11.5 `src/lib.rs`, trait `Emitter` |
| Window events | `WindowEvent` carries `Resized`, `Moved`, `CloseRequested`, `Destroyed`, `Focused(bool)`; `RunEvent::WindowEvent` carries the window label | `tauri-runtime` 2.11.3 `src/window.rs`; `tauri` 2.11.5 `src/app.rs` |
| Window geometry | `WindowConfig` carries `x`, `y`, `width`, `height` in logical pixels, `center` and `maximized`, applied by `from_config`; `outer_position` and `inner_size` read in physical pixels; `Monitor` carries `work_area` and `scale_factor`, and `AppHandle::available_monitors` lists them (re-read on 2026-09-26) | `tauri-utils` 2.9.3 `src/config.rs`; `tauri-runtime-wry` 2.11.4 `src/lib.rs`; `tauri` 2.11.5 `src/webview/webview_window.rs`, `src/window/mod.rs`, `src/app.rs` |
| PostgreSQL | `DROP TABLE` defaults to `RESTRICT`; `TRUNCATE` refuses a referenced table and is transactional; DDL is transactional except for database and tablespace | [sql-droptable](https://www.postgresql.org/docs/18/sql-droptable.html), [sql-truncate](https://www.postgresql.org/docs/18/sql-truncate.html), [wiki](https://wiki.postgresql.org/wiki/Transactional_DDL_in_PostgreSQL:_A_Competitive_Analysis) — documentation version 18 |
| Redshift | `TRUNCATE` "commits the transaction in which it is run"; `DROP TABLE` defaults to `RESTRICT` | [r_TRUNCATE](https://docs.aws.amazon.com/redshift/latest/dg/r_TRUNCATE.html), [r_DROP_TABLE](https://docs.aws.amazon.com/redshift/latest/dg/r_DROP_TABLE.html) |
| MySQL 8.4 | `DROP TABLE`, `TRUNCATE TABLE`, `RENAME TABLE`, `ALTER TABLE` commit implicitly | [implicit-commit](https://dev.mysql.com/doc/refman/8.4/en/implicit-commit.html) |
| SQLite | No `TRUNCATE`; `DROP TABLE` goes through despite a dependent view, and despite child rows when `foreign_keys` is `0`; `DROP` is undone by `ROLLBACK` | observed with the `sqlite3` client 3.51.0; the driver embeds 3.50.2, hence ADR-0042's requirement: a driver integration test before declaring each flag |

## SQL string literals — check of 2026-09-25

Facts that `push_string_literal` (`crates/oxyn-catalog/src/literal.rs`) relies
on. This function writes the values of the `INSERT` and `IN (…)` copies. A
wrong rule here lets a value close its literal once pasted
([I-10](../CLAUDE.md#i-10)). Read from the official documentation.

| Engine | Verified fact | Source |
|---|---|---|
| PostgreSQL 18 | `''` in a standard string. `E'…'` accepts `\\` and `\'`. With `standard_conforming_strings` `on` (default since 9.1), the backslash is literal outside `E'…'`; when `off`, it also escapes in an ordinary string | [sql-syntax-lexical](https://www.postgresql.org/docs/18/sql-syntax-lexical.html) |
| MySQL 8.4 | `''` or `\'`; sequences `\0 \' \" \b \n \r \t \Z \\ \% \_`. Under `NO_BACKSLASH_ESCAPES`, the backslash no longer escapes anything | [string-literals](https://dev.mysql.com/doc/refman/8.4/en/string-literals.html) |
| Snowflake | The backslash escapes; `''` or `\'` for the apostrophe; `\\` for the backslash | [data-types-text](https://docs.snowflake.com/en/sql-reference/data-types-text) |
| ClickHouse | "you need to escape at least `'` and `\` using escape codes `\'` (or: `''`) and `\\`" | [syntax](https://clickhouse.com/docs/sql-reference/syntax) |
| BigQuery | The backslash introduces the sequences `\\ \' \" \n \r \t` and others; an unknown sequence is an error. A single-quoted string cannot contain a line break. Two separate literals concatenate, so `''` is not an apostrophe. A backquoted identifier "Have the same escape sequences as string literals" | [lexical](https://docs.cloud.google.com/bigquery/docs/reference/standard-sql/lexical) |
| SQL Server | `''` in a string. `N'…'` is interpreted as Unicode; a string without `N` goes through the code page. The `bit` type is written `0` or `1` | [constants-transact-sql](https://learn.microsoft.com/en-us/sql/t-sql/data-types/constants-transact-sql), page updated on 2026-09-21 |
| SQLite | `''`; "C-style escapes using the backslash character are not supported" | [lang_expr](https://www.sqlite.org/lang_expr.html) |
| DuckDB | `''`; escape sequences only exist with the `E` prefix | [literal_types](https://duckdb.org/docs/current/sql/data_types/literal_types.html) |
| Oracle 23 | `''`; the page gives the backslash no role | [Literals](https://docs.oracle.com/en/database/oracle/oracle-database/23/sqlrf/Literals.html) |

**What Oxyn does with it.**

- MySQL, Snowflake, ClickHouse: doubled backslash, apostrophe as `''`. These
  two forms stay safe whatever `NO_BACKSLASH_ESCAPES` is.
- BigQuery: `\'`, doubled backslash, and line breaks and tabs as `\n`, `\r`,
  `\t`.
- PostgreSQL: `E'…'` as soon as a backslash is present.
- SQL Server: `N'…'`.
- SQLite, DuckDB, Oracle and unknown dialects: `''` only.

**Not checked.**

- **Redshift.** Neither the value of `standard_conforming_strings` nor support
  for `E'…'` appear in the pages read:
  [r_Literals](https://docs.aws.amazon.com/redshift/latest/dg/r_Literals.html),
  [r_Character_types](https://docs.aws.amazon.com/redshift/latest/dg/r_Character_types.html).
  The code applies the PostgreSQL form, `E'…'` as soon as a backslash is
  present. Without `E'…'`, the text fails to parse; it never closes its
  literal.
- **BigQuery identifiers.** `quote_identifier` in `Backtick` style doubles the
  backquote. But BigQuery reads a backquoted identifier with the escapes of a
  string: that doubling is worthless there, and a name ending with a backslash
  would escape the closing backquote. No BigQuery driver exists today; to fix
  before the first one.

## Webview clipboard — check of 2026-09-25

Fact that `writeClipboard` (`apps/desktop/src/lib/clipboard.ts`) relies on: a
copy whose text comes from the backend is written **within** the gesture, with
a `ClipboardItem` whose value is a promise, and not through `writeText` after
an `await`.

| Subject | Verified fact | Source |
|---|---|---|
| Gesture required | "The request to write to the clipboard must be triggered during a user gesture."; each `ClipboardItem` is initialized with "a mapping of MIME type to `Promise` which may resolve either to a string or a `Blob`"; available since Safari 13.1 | WebKit, [Async Clipboard API](https://webkit.org/blog/10855/async-clipboard-api/), post of 2020-06-23 |
| Pattern for WebKit | "Safari (WebKit) treats user activation differently than Chromium (Blink). For Safari, run all asynchronous operations in a promise whose result you assign to the `ClipboardItem`"; the example resolves a `Blob` | web.dev, [Unblocking clipboard access](https://web.dev/articles/async-clipboard) |
| Symptom | `writeText()` rejected with `NotAllowedError` on Safari for lack of recognized activation, reported on Safari 18.3 (macOS 15.3.1); no answer from Apple | Apple Developer Forums, [thread 772275](https://developer.apple.com/forums/thread/772275), opened in January 2025 |
| Engines concerned | The minimum target is macOS 13.0 (`minimumSystemVersion`), shipped with Safari 16, later than 13.1; the dev machine is on macOS 26.2, Safari 26.2 | `sw_vers` and Safari's `Info.plist`, read on 2026-09-25; `mermaid` table above for macOS 13 |

Resulting choices: the value of the item is a `text/plain` `Blob`, the form
both sources show; a text already known goes through `writeText`, within the
gesture. Where `ClipboardItem` is missing, the text is awaited then written
through `writeText`, and a `NotAllowedError` there becomes a message saying the
copy arrived after the click.

**Not reproduced**: no test reaches the application's WKWebView — the stories
run under Chromium, and Playwright's WebKit is not installed here. Manual check
in `make desktop-dev`: `Copy as ▸ INSERT template` on a table never opened,
then paste.

## Licenses — check of 2026-09-25

Facts that [ADR-0044](adr/0044-licence-gpl-et-contrat-apache.md) relies on.

### The repository

| Fact | Value | Source |
|---|---|---|
| Repository | `so-keyldzn/oxyn`, private, license detected by GitHub: Apache-2.0 before this change | `gh repo view so-keyldzn/oxyn` |
| Former value of `repository` | `https://github.com/keyldzn/oxyn`, which does not resolve | `gh repo view keyldzn/oxyn`: "Could not resolve to a Repository" |

### The text of the GPL

| Fact | Value | Source |
|---|---|---|
| `LICENSE-GPL` | GNU General Public License, version 3, 29 June 2007; 674 lines; SHA-256 `3972dc9744f6499f0f9b2dbf76696f2ae7ad8af9b23dde66d6af86c9dfb36986` | downloaded as is from <https://www.gnu.org/licenses/gpl-3.0.txt> on 2026-09-25 |

### GPLv3 compatibility of the accepted licenses

Read in the FSF list, <https://www.gnu.org/licenses/license-list.html>, on
2026-09-25. The anchor is that of the entry in the page.

| SPDX license | FSF entry | Verdict |
|---|---|---|
| `Apache-2.0` | `#apache2` | compatible with GPLv3 (not with GPLv2) |
| `Apache-2.0 WITH LLVM-exception` | — | the exception adds permissions to Apache-2.0 and removes none |
| `MIT` | `#Expat`, `#X11License` | compatible |
| `MIT-0` | `#Expat0` | compatible, like `#Zero-BSD` |
| `BSD-2-Clause` | `#FreeBSD` | compatible |
| `BSD-3-Clause` | `#ModifiedBSD` | compatible |
| `ISC` | `#ISC` | compatible |
| `Zlib` | `#ZLib` | compatible |
| `Unicode-3.0` | `#Unicodev3` | compatible with all versions of the GPL |
| `CC0-1.0` | `#CC0` | compatible |
| `MPL-2.0` | `#MPL-2.0` | compatible through section 3.3, except for a file marked "Incompatible With Secondary Licenses" |
| `BSL-1.0` | `#boost` | compatible |
| `NCSA` | `#NCSA` | compatible |
| `CDLA-Permissive-2.0` | absent from the list | see below |
| `0BSD` (npm) | `#Zero-BSD` | compatible |
| `Unlicense` (npm) | `#Unlicense` | compatible |
| `Python-2.0` (npm) | `#Python` | compatible (versions 2.0.1, 2.1.1 and later) |
| `CC-BY-4.0` (npm) | `#ccby` | compatible with all versions of the GPL, and not to be used for software |
| `OFL-1.1` (npm) | `#SILOFL` | free license for fonts. Its only unusual requirement, selling the font with software and not alone, is "harmless" according to the FSF |

**`CDLA-Permissive-2.0`**, read in the SPDX text
(<https://github.com/spdx/license-list-data>, `text/CDLA-Permissive-2.0.txt`,
on 2026-09-25). The license covers data. Its only sharing condition is § 2.1:
"makes available the text of this agreement with the shared Data". § 3.1
imposes nothing on results. It is a notice to reproduce, which GPLv3 allows
requiring (§ 7 b). **This conclusion is ours, not the FSF's.** The
application's third-party notices reproduce this text.

**`MPL-2.0` in the Rust graph.** The crates concerned are `cssparser` 0.36.0,
`cssparser-macros` 0.6.1, `dtoa-short` 0.3.5, `option-ext` 0.2.0 and
`selectors` 0.36.1, all pulled by Tauri. None of their `.rs` files carries the
"Incompatible With Secondary Licenses" notice. The search was done in
`~/.cargo/registry/src` on 2026-09-25; the only occurrences are in the text of
the license itself, which cites the notice as a template.

### The CLA template

| Fact | Value | Source |
|---|---|---|
| Template | Apache Software Foundation, *Individual Contributor License Agreement* V2.2 | <https://www.apache.org/licenses/icla.pdf>, downloaded on 2026-09-25 |
| License granted (§ 2) | "perpetual, worldwide, non-exclusive, no-charge, royalty-free, irrevocable copyright license to reproduce, prepare derivative works of, publicly display, publicly perform, sublicense, and distribute" | same document |
| Patent license (§ 3) | limited to the claims necessarily infringed by the contribution; terminated for whoever files an infringement action | same document |

The differences between `CLA.md` and this template are listed at the end of
`CLA.md`. The right to sublicense ("sublicense") is what allows relicensing;
§ 2 of `CLA.md` spells it out. The assignment (§ 9) does not exist in the
template. **This text has not been reviewed by a lawyer**: the review is to be
done with the assignment of the rights to the company.

### Third-party notices

| Fact | Value | Source | Checked on |
|---|---|---|---|
| `cargo-about` | **0.9.2**, published on 2026-08-18; `MIT OR Apache-2.0`; `rust-version` 1.88.0 | [crates.io](https://crates.io/crates/cargo-about) | 2026-09-25 |
| Binaries of `cargo-about` 0.9.2 | `aarch64-apple-darwin`: SHA-256 `ae72f0df0c399a1e96336f696fa55b1b28679fd725632eba8cf8e4568467cc3e`; `x86_64-unknown-linux-musl`: `9099a59e820c38a68b9d65f300662a567d56562f9a10f6aa4c7e86c17c2566af`. No `x86_64-apple-darwin` binary | [GitHub release](https://github.com/EmbarkStudios/cargo-about/releases/tag/0.9.2), `.sha256` files recomputed on download | 2026-09-25 |
| `private = { ignore = true }` | exists in the configuration of `cargo-about` as in that of `cargo-deny`; without it, each GPL crate of the workspace makes generation fail ("failed to satisfy license requirements") | running `cargo about generate` | 2026-09-25 |
| `pnpm licenses list --prod --json` | an object `{ licence: [{ name, versions, paths, license, … }] }`; `Unknown` when `package.json` has no `license` field; works with pnpm 11.1.2 | run in `apps/desktop` | 2026-09-25 |

Three observations from the first generation, on 2026-09-25:

- 417 crates and 413 npm packages, 297 distinct texts, 1.18 MB of JSON. Vite
  makes it a separate chunk, 100 kB compressed, loaded when the About section
  opens;
- `cargo about generate` takes about 20 s and 100 s of CPU, mostly to
  identify the license texts. This is why `make front-build` only regenerates
  the file if `Cargo.lock`, `pnpm-lock.yaml`, `deny.toml` or the npm
  configuration changed;
- eleven npm packages ship no license file, including `@uiw/react-codemirror`
  and `embla-carousel`. The notices then take the declared SPDX expression.

## External contracts cross-checked during the audit — 2026-09-24

Consulted on **24 September 2026**. No dependency was changed. Local versions
were read in the manifests, `Cargo.lock` and the installed sources: Rust
1.98.1, Tokio 1.53.1, reqwest 0.13.4, SQLx 0.9.0, rusqlite 0.37.0,
libsqlite3-sys 0.35.0 (embedded SQLite 3.50.2), Tauri 2.11.5, React 19.3.0,
TanStack Query 5.102.8, Store 0.11.1, Base UI 1.8.0, Shiki 4.4.3. `latest` or
`main` documents were cross-checked with the installed sources for the
behaviors used. PostgreSQL 18 below designates the documentation version
consulted, **not** the version of a tested server.

| Subject | Verified external fact | Official source |
|---|---|---|
| HTTP redirection | Statuses 307/308 keep the method during an automatic redirection; the client must control its destination | [RFC 9110, 307](https://www.rfc-editor.org/rfc/rfc9110.html#section-15.4.8), [308](https://www.rfc-editor.org/rfc/rfc9110.html#section-15.4.9) |
| reqwest | The default policy follows redirections; header removal across origins does not cover the proprietary names `x-api-key` and `api-key` | [source v0.13.4, redirect.rs](https://github.com/seanmonstar/reqwest/blob/v0.13.4/src/redirect.rs) |
| HTTP diagnostics | `Response::text` collects the body; a truncation applied afterwards does not bound that read | [reqwest 0.13.4](https://docs.rs/reqwest/0.13.4/reqwest/struct.Response.html#method.text) |
| Anthropic stream | Errors can arrive in the stream; tool arguments arrive as JSON fragments | [SSE errors](https://platform.claude.com/docs/en/build-with-claude/streaming#error-events), [JSON deltas](https://platform.claude.com/docs/en/build-with-claude/streaming#input-json-delta) |
| PostgreSQL | A `VOLATILE` function can modify the database; `READ ONLY` protects non-temporary tables and is not a universal containment of side effects | [volatility](https://www.postgresql.org/docs/18/xfunc-volatility.html), [transactions](https://www.postgresql.org/docs/18/sql-set-transaction.html) |
| SQLite | `sqlite3_stmt_readonly` classifies transaction commands as non-mutating; this does not guarantee ownership of the transaction | [readonly](https://www.sqlite.org/c3ref/stmt_readonly.html), [transactions](https://www.sqlite.org/lang_transaction.html) |
| Tokio | `timeout` only bounds its future, returns an error on expiry and does not preempt a future that does not yield | [Tokio 1.53.1](https://docs.rs/tokio/1.53.1/tokio/time/fn.timeout.html) |
| Rust file | `File::create` immediately truncates an existing file | [Rust 1.98.1](https://doc.rust-lang.org/std/fs/struct.File.html#method.create) |
| React | An asynchronous handler keeps the captured state; a mutable ref persists; changing the key resets the subtree | [snapshot](https://react.dev/learn/state-as-a-snapshot#state-over-time), [useRef](https://react.dev/reference/react/useRef#reference), [key](https://react.dev/learn/preserving-and-resetting-state#resetting-state-with-a-key) |
| TanStack Store | The store exists outside the React cycle; its updates are explicit | [Quick Start](https://tanstack.com/store/latest/docs/framework/react/quick-start) |
| TanStack Query | `staleTime: Infinity` keeps freshness until invalidation; invalidation is targeted by key | [defaults](https://tanstack.com/query/latest/docs/framework/react/guides/important-defaults), [invalidation](https://tanstack.com/query/latest/docs/framework/react/guides/query-invalidation) |
| Base UI | `Tabs.Panel.keepMounted` keeps the hidden panel in the DOM | [Tabs.Panel](https://base-ui.com/react/components/tabs#panel) |
| Tauri IPC | Arguments reach the handler and its response is serialized; this replaces neither business validation nor keeping a Rust reader | [arguments](https://v2.tauri.app/develop/calling-rust/#passing-arguments), [return](https://v2.tauri.app/develop/calling-rust/#returning-data) |
| Shiki/TextMate | The tokenization timeout is expressed per line; TextMate can interrupt the pass. Shiki's public result does not directly expose `stoppedEarly` | [Shiki types](https://github.com/shikijs/shiki/blob/main/packages/types/src/tokens.ts), [TextMate tokenization](https://github.com/microsoft/vscode-textmate/blob/main/src/grammar/tokenizeString.ts) |
| Tauri version | The configured version determines the application's, independently of the name of a Git tag | [configuration](https://v2.tauri.app/reference/config/#version) |
| GitHub CLI | `--verify-tag` checks the tag exists; `isDraft` can be queried; `upload --clobber` deletes the old asset before uploading | [create](https://cli.github.com/manual/gh_release_create), [view](https://cli.github.com/manual/gh_release_view), [upload](https://cli.github.com/manual/gh_release_upload) |

These sources establish the contracts of the dependencies. Oxyn's own defects,
the synthetic reproductions and their limits are recorded in the dated reports
of `.claude/audits/` and the corresponding issues; an official link alone does
not constitute a reproduction of the product.

## `sqlx` and PostgreSQL types — checked on 2026-09-28

`sqlx` 0.9.0, the latest release on crates.io on 2026-09-28 (published
2026-05-21), used by `oxyn-driver-postgres`:

| Fact | Consequence | Source |
|---|---|---|
| After Describe, `resolve_statement_metadata` queries `pg_type` for each unknown OID and accepts only `typtype` `b c d e p r` and `typcategory` `A B C D E G I N P R S T U V X`; the `main` branch is the same | multiranges (`'m'`, PostgreSQL 14+) and internal types (`'Z'`: `pg_node_tree`, `pg_ndistinct`, `pg_dependencies`, `pg_mcv_list`, BRIN summaries) fail `prepare` with `ColumnDecode` on `typtype` / `typcategory` | [resolve.rs](https://github.com/launchbadge/sqlx/blob/main/sqlx-postgres/src/connection/resolve.rs) |
| The Bind of `query_with` asks for the binary format for every result column, hard-coded | `aclitem`, `gtsvector` and their arrays (`typsend = 0` in `pg_type` of PostgreSQL 17.11) are refused by the server: `no binary output function available` | [executor.rs](https://github.com/launchbadge/sqlx/blob/main/sqlx-postgres/src/connection/executor.rs) |
| `raw_sql` runs the simple protocol and does not resolve unknown OIDs (`DeclareWithOid`) | every value arrives as text; `PgTypeInfo::name()` returns `"?"` for such a type | same |
| `PgTypeInfo::kind()` calls `unreachable!` on an unresolved type | a simple-protocol column must never go through the typed decoding (ADR-0048) | [type_info.rs](https://github.com/launchbadge/sqlx/blob/main/sqlx-postgres/src/type_info.rs) |

## PostgreSQL binary wire formats — checked on 2026-09-28

What the server sends for each type in the extended protocol's **binary**
format (the only one the driver receives), and the canonical text the
`*_out` function prints for the same value. Read in the sources of
`REL_18_STABLE`, compared with `REL_12_STABLE` (the oldest version the driver
supports); a difference is stated in the row. Every integer and every
`float8` is big-endian (`pq_sendint*`, `pq_sendfloat8`).

| Type | Binary send layout | Text output | Source |
|---|---|---|---|
| `inet`, `cidr` | `u8` family (`PGSQL_AF_INET` = `AF_INET`+0 = 2, `PGSQL_AF_INET6` = 3), `u8` bits, `u8` is_cidr, `u8` nb (4 or 16), then nb address bytes | IPv4: four decimal octets, `/bits` only if bits ≠ 32. IPv6: lowercase hex words, longest run (≥ 2) of zero words as `::`, embedded IPv4 when the first 6 words are 0, or 7 are 0 and the last ≠ 1, or it is `::ffff:` mapped; `/bits` only if bits ≠ 128. `cidr` always appends `/bits` | [network.c](https://github.com/postgres/postgres/blob/REL_18_STABLE/src/backend/utils/adt/network.c) `network_send`, `network_out`; [inet_net_ntop.c](https://github.com/postgres/postgres/blob/REL_18_STABLE/src/port/inet_net_ntop.c) |
| `macaddr` | 6 bytes | `%02x` × 6, `:`-separated, lowercase | [mac.c](https://github.com/postgres/postgres/blob/REL_18_STABLE/src/backend/utils/adt/mac.c) |
| `macaddr8` | 8 bytes | `%02x` × 8, `:`-separated, lowercase | [mac8.c](https://github.com/postgres/postgres/blob/REL_18_STABLE/src/backend/utils/adt/mac8.c) |
| `bit`, `varbit` | `i32` bit length, then ⌈len/8⌉ bytes, most significant bit first, padding bits zero | one `0`/`1` per bit | [varbit.c](https://github.com/postgres/postgres/blob/REL_18_STABLE/src/backend/utils/adt/varbit.c) `varbit_send` (shared by `bit_send`) |
| `point` | `f64` x, `f64` y | `(x,y)` | [geo_ops.c](https://github.com/postgres/postgres/blob/REL_18_STABLE/src/backend/utils/adt/geo_ops.c) |
| `line` | `f64` A, B, C | `{A,B,C}` | same |
| `lseg` | `f64` x1, y1, x2, y2 | `[(x1,y1),(x2,y2)]` | same |
| `box` | `f64` high.x, high.y, low.x, low.y — **upper-right corner first** | `(high.x,high.y),(low.x,low.y)` | same |
| `path` | `u8` closed (1/0), `i32` npts, then npts × (`f64` x, `f64` y) | closed `((x,y),…)`, open `[(x,y),…]` | same |
| `polygon` | `i32` npts, then npts × (`f64` x, `f64` y) | `((x,y),…)` | same |
| `circle` | `f64` center.x, center.y, radius | `<(x,y),r>` | same |
| geometric floats | — | `float8out_internal`: with `extra_float_digits` = 1 (default in 12 and 18) the shortest round-trip digits; fixed notation when the decimal exponent is in [-4, 15), otherwise `d.ddde+XX` (at least two exponent digits); `NaN`, `Infinity`, `-Infinity`, `-0` | [float.c](https://github.com/postgres/postgres/blob/REL_18_STABLE/src/backend/utils/adt/float.c), [d2s.c](https://github.com/postgres/postgres/blob/REL_18_STABLE/src/common/d2s.c), [ryu_common.h](https://github.com/postgres/postgres/blob/REL_18_STABLE/src/common/ryu_common.h) |
| `pg_lsn` | `u64` | `%X/%X`: high 32 bits / low 32 bits, uppercase hex, no zero padding. **PostgreSQL 19** prints `%X/%08X`, the low half padded to eight digits ([pg_lsn.c](https://github.com/postgres/postgres/blob/REL_19_STABLE/src/backend/utils/adt/pg_lsn.c)); the driver prints the ≤ 18 form, which every version reads back | [pg_lsn.c](https://github.com/postgres/postgres/blob/REL_18_STABLE/src/backend/utils/adt/pg_lsn.c) |
| `xid`, `cid` | `u32` | decimal | [xid.c](https://github.com/postgres/postgres/blob/REL_18_STABLE/src/backend/utils/adt/xid.c) |
| `xid8` | `u64` | decimal. **PostgreSQL 13+** | same |
| `tid` | `u32` block, `u16` offset | `(block,offset)` | [tid.c](https://github.com/postgres/postgres/blob/REL_18_STABLE/src/backend/utils/adt/tid.c) |
| `pg_snapshot`, `txid_snapshot` | `i32` nxip, `u64` xmin, `u64` xmax, then nxip × `u64` — **count first** | `xmin:xmax:xip1,xip2,…` | [xid8funcs.c](https://github.com/postgres/postgres/blob/REL_18_STABLE/src/backend/utils/adt/xid8funcs.c); 12: `txid.c`, same layout. `pg_snapshot` is **13+** |
| `tsvector` | `i32` lexeme count; per lexeme: text in the client encoding, `\0`, `u16` npos, npos × `u16` (weight in bits 15–14: 3 = A, 2 = B, 1 = C, 0 = D; position in bits 13–0) | `'lexeme'` with `'` and `\` doubled, then `:pos` + weight letter (D not printed), `,`-separated; lexemes separated by a space | [tsvector.c](https://github.com/postgres/postgres/blob/REL_18_STABLE/src/backend/utils/adt/tsvector.c), [ts_type.h](https://github.com/postgres/postgres/blob/REL_18_STABLE/src/include/tsearch/ts_type.h) |
| `tsquery` | `i32` item count; items in **prefix order**: `u8` type (1 = operand, 2 = operator); operand: `u8` weight bitmask (A = 8, B = 4, C = 2, D = 1), `u8` prefix, `\0`-terminated text; operator: `u8` oper (1 NOT, 2 AND, 3 OR, 4 PHRASE), then `i16` distance for PHRASE only | infix: after a binary operator comes the **right** operand, then the left; prints `left & right`, `\|`, `<->` (distance 1) or `<N>`, `!x`. Priorities NOT 4 > PHRASE 3 > AND 2 > OR 1; `( … )` when the child's priority is lower than the parent's, or for a PHRASE on the right of a PHRASE. Operand: quoted like `tsvector`, then `:*` if prefix and `ABCD` letters of the weight. Empty query: empty string | [tsquery.c](https://github.com/postgres/postgres/blob/REL_18_STABLE/src/backend/utils/adt/tsquery.c) `tsquerysend`, `infix` |
| ranges | `u8` flags (`0x01` empty, `0x02` lower inclusive, `0x04` upper inclusive, `0x08` lower infinite, `0x10` upper infinite); then, when present, `i32` length + element binary for the lower bound, same for the upper | `empty`, or `[`/`(` lower `,` upper `]`/`)`; an absent bound prints nothing; a bound is quoted `"…"` if empty or containing `"` `\` `(` `)` `[` `]` `,` or a space, with `"` and `\` doubled | [rangetypes.c](https://github.com/postgres/postgres/blob/REL_18_STABLE/src/backend/utils/adt/rangetypes.c), [rangetypes.h](https://github.com/postgres/postgres/blob/REL_18_STABLE/src/include/utils/rangetypes.h) |
| multiranges | `i32` range count, then per range `i32` length + range binary | `{range,range}`, `{}` when empty. **PostgreSQL 14+** | [multirangetypes.c](https://github.com/postgres/postgres/blob/REL_18_STABLE/src/backend/utils/adt/multirangetypes.c) |
| `record`, composite | `i32` count of non-dropped columns; per column `u32` type OID, `i32` length (-1 = `NULL`), bytes | `(v1,v2)`; `NULL` prints nothing; quoted like a range bound except `[` `]` do not force quotes | [rowtypes.c](https://github.com/postgres/postgres/blob/REL_18_STABLE/src/backend/utils/adt/rowtypes.c) `record_send`, `record_out` |
| array nested in a value | `i32` ndim (0 = empty), `i32` flags, `u32` element OID, then ndim × (`i32` length, `i32` lower bound), then the elements row-major, each `i32` length (-1 = `NULL`) + bytes; at most `MAXDIM` = 6 dimensions | `[lb:ub]` per dimension then `=` when **any** lower bound ≠ 1; nested `{…}` per dimension; elements separated by `typdelim` — `;` for `box`, the only built-in exception, `,` otherwise; an element is quoted when empty, equal to `NULL` (case-insensitive), or containing `{` `}` `"` `\` the delimiter or a space, with `"` and `\` backslash-escaped | [arrayfuncs.c](https://github.com/postgres/postgres/blob/REL_18_STABLE/src/backend/utils/adt/arrayfuncs.c) `array_send`, `array_out`; [array.h](https://github.com/postgres/postgres/blob/REL_18_STABLE/src/include/utils/array.h) `MAXDIM`; [pg_type.dat](https://github.com/postgres/postgres/blob/REL_18_STABLE/src/include/catalog/pg_type.dat) `typdelim` |
| built-in OID tables of `types.rs` | `BUILTIN_ARRAYS`: 71 (array, element) pairs, `typarray` / `array_type_oid` of every non-pseudo type; `BUILTIN_RANGES` and `BUILTIN_MULTIRANGES`: the six ranges (3904, 3906, 3908, 3910, 3912, 3926) and multiranges (4451, 4532–4536) with their subtype; `NO_BINARY_OUTPUT`: the non-pseudo types with `typsend = '-'` — `aclitem` 1033, `gtsvector` 3642 — and their arrays 1034, 3644 | built-in OIDs are fixed in the catalog sources; cross-checked against `pg_type` of PostgreSQL 17.11 | [pg_type.dat](https://github.com/postgres/postgres/blob/REL_18_STABLE/src/include/catalog/pg_type.dat), [pg_range.dat](https://github.com/postgres/postgres/blob/REL_18_STABLE/src/include/catalog/pg_range.dat) |
| `money` | `i64` in minor units | the number of decimals comes from the server's `lc_monetary` (`frac_digits`, 2 if out of range): **not on the wire** | [cash.c](https://github.com/postgres/postgres/blob/REL_18_STABLE/src/backend/utils/adt/cash.c) |
| `void` | zero bytes | empty string | [pseudotypes.c](https://github.com/postgres/postgres/blob/REL_18_STABLE/src/backend/utils/adt/pseudotypes.c) |
| `date` | `i32` days since 2000-01-01; `i32::MIN` / `i32::MAX` = `-infinity` / `infinity` | `j2date(days + 2451545)`, then (ISO) `YYYY-MM-DD`, year zero-padded to 4 digits; year ≤ 0 prints `1 - year` and appends ` BC` | [date.c](https://github.com/postgres/postgres/blob/REL_18_STABLE/src/backend/utils/adt/date.c) `date_out`; [datetime.c](https://github.com/postgres/postgres/blob/REL_18_STABLE/src/backend/utils/adt/datetime.c) `j2date`, `EncodeDateOnly` |
| `time` | `i64` microseconds since midnight, `24:00:00` allowed | `HH:MM:SS`, then `.` and up to 6 fraction digits **without trailing zeros** (`AppendSeconds`), nothing if the fraction is 0 | [date.c](https://github.com/postgres/postgres/blob/REL_18_STABLE/src/backend/utils/adt/date.c) `time_out`; [datetime.c](https://github.com/postgres/postgres/blob/REL_18_STABLE/src/backend/utils/adt/datetime.c) `EncodeTimeOnly`, `AppendSeconds` |
| `timestamp`, `timestamptz` | `i64` microseconds since 2000-01-01 (UTC for `timestamptz`); `i64::MIN` / `i64::MAX` = `-infinity` / `infinity` | split by Euclidean division into a Julian day and a time of day (`timestamp2tm`), then (ISO) `YYYY-MM-DD HH:MM:SS[.frac]`; `timestamptz` then appends the session offset — with `TimeZone = 'UTC'`, `+00` (`EncodeTimezone` prints `:MM` only for a non-zero minute); ` BC` comes **last**, after the offset | [timestamp.c](https://github.com/postgres/postgres/blob/REL_18_STABLE/src/backend/utils/adt/timestamp.c) `timestamp_out`, `timestamp2tm`; [datetime.c](https://github.com/postgres/postgres/blob/REL_18_STABLE/src/backend/utils/adt/datetime.c) `EncodeDateTime`, `EncodeTimezone` |
| `interval` | `i64` microseconds, `i32` days, `i32` months; all three at their minimum / maximum = `-infinity` / `infinity` (**PostgreSQL 17+**; a single extreme field is finite) | `IntervalStyle = 'postgres'` (default): years = months / 12, months = months % 12 (truncating), then `N year(s)`, `N mon(s)`, `N day(s)` for non-zero fields (plural unless 1, `+` before a positive field that follows a negative one); then `[-\|+]HH:MM:SS[.frac]` if the time is non-zero or nothing was printed, `-` if any time part is negative | [timestamp.c](https://github.com/postgres/postgres/blob/REL_18_STABLE/src/backend/utils/adt/timestamp.c) `interval_out`, `interval2itm`; [datetime.c](https://github.com/postgres/postgres/blob/REL_18_STABLE/src/backend/utils/adt/datetime.c) `EncodeInterval`, `AddPostgresIntPart`; [timestamp.h](https://github.com/postgres/postgres/blob/REL_18_STABLE/src/include/datatype/timestamp.h) `INTERVAL_NOBEGIN` |

## YAML parsing for agent files — checked on 2026-09-29

Read for [ADR-0049](adr/0049-agents-declared-as-markdown-files.md), on the
crates.io API and in the sources of `serde-saphyr` 1.3.0
(`src/de/budget.rs`, `src/de/options.rs`, `README.md`).

| Crate | Latest stable | Published / updated | State | Source |
|---|---|---|---|---|
| `serde-saphyr` | `1.3.0`, MIT OR Apache-2.0, `rust-version = "1.89"` | 2026-09-16 | maintained; panic-free parsing is a stated goal; no tag-driven object construction | crates.io API; [repository](https://github.com/bourumir-wyngs/serde-saphyr) |
| `serde_yaml` | `0.9.34+deprecated` | 2024-03-25 | deprecated by its author | crates.io API |
| `serde_yml` | `0.0.13` | 2026-05-27 | its own description: "DEPRECATED — unmaintained", a compatibility shim | crates.io API |
| `serde_norway` | `0.9.42` | 2024-12-21 | fork of `serde_yaml`, no release since | crates.io API |
| `toml` (already in the workspace at `0.9.8`) | `1.1.6+spec-1.1.0` | — | the fallback format of ADR-0049 | crates.io API |

`serde-saphyr` 1.3.0 settings ADR-0049 relies on:

| Setting | Default | Meaning |
|---|---|---|
| `Budget::max_anchors`, `Budget::max_aliases` | 50,000 each | `0` refuses the first anchor / alias |
| `Budget::max_reader_input_bytes` | 256 MiB | applies to reader input only, not to a `&str` — the 64 KiB cap is checked by the caller |
| `Options::merge_keys` | `MergeKeyPolicy::Merge` | `Error` refuses `<<` |
| `Options::duplicate_keys` | `DuplicateKeyPolicy::Error` | a repeated key is an error |
| `Options::strict_booleans` | `false`: `yes`/`no`/`on`/`off` read as booleans | `true` accepts only `true` / `false` |
| `Options::reject_unsupported_tags` | `false` | `true` refuses an unknown tag |
| `!include` | only with the `include` feature and a resolver | left off |

## MySQL protocol and client libraries — checked on 2026-09-30

For [ADR-0050](adr/0050-mysql-driver-on-mysql-async-prepared-first.md).

| Fact | Consequence | Source |
|---|---|---|
| MySQL LTS lines: 8.4 (8.4.11, EOL 2032-04-30) and 9.7 (9.7.2, released 2026-04-21, EOL 2034-04-30); 8.0 (8.0.46) reached EOL on 2026-04-30. The `trunk` branch reads 26.10.0, "INNOVATION", previous LTS 9.7.0 | the driver is tested on 8.4 and 9.7 | [endoflife.date/mysql](https://endoflife.date/mysql); [MYSQL_VERSION](https://github.com/mysql/mysql-server/blob/trunk/MYSQL_VERSION) |
| MariaDB LTS lines: 11.8 (11.8.9, EOL 2028-06-04) and 12.3 (12.3.3, EOL 2029-06-12); 13.0 (13.0.2) is a short-term release | the driver is tested on 11.8 and 12.3 | [endoflife.date/mariadb](https://endoflife.date/mariadb) |
| `MYSQL_TYPE_VECTOR = 242`; `Field_vector::type()` returns it, so a `VECTOR` column is announced with that code | a client must know code 242 | [field_types.h](https://github.com/mysql/mysql-server/blob/trunk/include/field_types.h), [field.h](https://github.com/mysql/mysql-server/blob/trunk/sql/field.h) |
| `sqlx` 0.9.0 and `sqlx-mysql` 0.9.0 are the latest releases (2026-05-21). `ColumnType::try_from_u16` has no arm for 0xf2 and returns `unknown column type`; `main` is the same | `sqlx-mysql` fails a result holding a `VECTOR` | `sqlx-mysql` 0.9.0 `src/protocol/text/column.rs`; [main](https://github.com/launchbadge/sqlx/blob/main/sqlx-mysql/src/protocol/text/column.rs) |
| `sqlx-mysql` 0.9.0: `MySqlTypeInfo` keeps type, flags, collation and `max_size`, not `decimals`; `MySqlValueRef::as_bytes` and `MySqlConnection::in_transaction` are `pub(crate)`; `AuthPlugin` knows `mysql_native_password`, `caching_sha2_password`, `sha256_password`, `mysql_clear_password` only | no `DECIMAL` scale, no MariaDB `client_ed25519`/`parsec` | `src/type_info.rs`, `src/value.rs`, `src/connection/mod.rs`, `src/protocol/auth.rs` |
| `sqlx-mysql`'s full `caching_sha2_password` authentication: over TLS, the password in the clear inside the tunnel; without TLS, the `rsa` feature, which pulls `rsa` 0.10.0-rc.18 (latest stable 0.9.10); RUSTSEC-2023-0071 (Marvin attack) lists `patched = []` | `sqlx-mysql` without TLS needs an unpatched advisory | `src/connection/auth.rs`; crates.io API; RustSec advisory database |
| `mysql_async` 0.37.1 (2026-09-01, MIT OR Apache-2.0) depends on `mysql_common` ^0.37.1 (latest 0.37.x: 0.37.3; latest: 0.38.2, 2026-07-24). `mysql_common` 0.37 has `MYSQL_TYPE_VECTOR`, `Column::decimals`, `column_length`, `character_set`, `flags`, and the `client_ed25519` / `client_parsec` features, which `mysql_async` re-exports. Its RSA uses `num-bigint`, not the `rsa` crate | `mysql_async` reads every column and authenticates MariaDB's plugins | [mysql_async Cargo.toml](https://github.com/blackbeam/mysql_async/blob/v0.37.1/Cargo.toml); [rust_mysql_common](https://github.com/blackbeam/rust_mysql_common) `src/constants.rs`, `src/packets/mod.rs`, `Cargo.toml` |
| `mysql_async` features: `default-rustls` = `rustls-tls` + `aws-lc-rs` + `tls12`; `Conn::id()`, `Conn::last_ok_packet()` are public; with `stmt_cache_size` at `0`, "you must close statements manually" | TLS on the workspace's provider; `KILL QUERY` target; transaction status from the server | `Cargo.toml`, `src/conn/mod.rs`, `src/opts/mod.rs` at v0.37.1 |
| `sqlx-mysql` announces `MULTI_STATEMENTS`, `MULTI_RESULTS`, `PS_MULTI_RESULTS` unconditionally (`src/connection/stream.rs`); `mysql_async` announces `CLIENT_MULTI_STATEMENTS` and `CLIENT_LOCAL_FILES` unconditionally (`Opts::get_capabilities`); neither sends `COM_SET_OPTION` | `COM_QUERY` runs a multi-statement text whatever the library; with `mysql_async`, only the missing handler refuses a `LOCAL INFILE` request | `sqlx-mysql` 0.9.0; [opts/mod.rs](https://github.com/blackbeam/mysql_async/blob/master/src/opts/mod.rs) |
| MySQL 8.4 `Prepared_statement::prepare` sets `m_lip.multi_statements = false` before `parse_sql`; the grammar then accepts only `END_OF_INPUT` after `;`. `ER_UNSUPPORTED_PS` is raised later, in `prepare_query` | a multi-statement text fails the prepare with 1064 before any 1295 | [sql_prepare.cc](https://github.com/mysql/mysql-server/blob/8.4/sql/sql_prepare.cc), [sql_yacc.yy](https://github.com/mysql/mysql-server/blob/8.4/sql/sql_yacc.yy) `sql_statement` |
| `prepare_query`'s switch accepts, among others, `SET` (`SQLCOM_SET_OPTION`), `CREATE VIEW` (not `ALTER VIEW`), `COMMIT`, `ROLLBACK`, DDL on tables and indexes, DML, `CALL`, `SHOW`; the `default` arm refuses the rest with 1295 — `BEGIN`, `SAVEPOINT`, `USE`, `LOCK TABLES`, routine, trigger and event creation, `LOAD DATA`, `XA`. The manual's list is narrower than the code | the text fallback serves a closed set of statements | same; [sql-prepared-statements](https://dev.mysql.com/doc/refman/8.4/en/sql-prepared-statements.html) |
| `ER_UNSUPPORTED_PS` 1295 (HY000); `ER_PARSE_ERROR` 1064 (42000); `ER_QUERY_INTERRUPTED` 1317 (70100) | 1295 alone opens the fallback | [server-error-reference](https://dev.mysql.com/doc/mysql-errors/8.4/en/server-error-reference.html) |
| `net_write_timeout` defaults to 60 s: the server aborts a write the client does not read; `max_execution_time` applies to `SELECT` only; `time_zone` defaults to `SYSTEM` | cancel an abandoned result; set `time_zone` per session | [server-system-variables](https://dev.mysql.com/doc/refman/8.4/en/server-system-variables.html) |
| `KILL QUERY` ends the running statement and keeps the connection; the flag is read after each block of rows; without `CONNECTION_ADMIN` or `SUPER`, only one's own threads | cancellation from a second connection of the same account | [kill](https://dev.mysql.com/doc/refman/8.4/en/kill.html) |
| `READ ONLY` access mode prohibits DDL and DML on permanent tables; DML on `TEMPORARY` tables stays allowed | the observation below shows no temporary table can be created in that mode | [set-transaction](https://dev.mysql.com/doc/refman/8.4/en/set-transaction.html) |
| `oxyn-query` at `origin/main` of 2026-09-30, `SqlDialect::MySql`: `CREATE PROCEDURE p() BEGIN SELECT 1; DELETE FROM t; END` splits into three fragments; `/*!80000 DELETE FROM t */` classifies `Unknown` with no fragment; `SELECT 1 /*!80000 ; DELETE FROM t */` classifies `Write` / `UnboundedDelete` | the scanner must learn MySQL compound bodies before the driver ships | probe run against `oxyn_query::split` and `classify` |
| Pinned: `mysql_async` `=0.37.1` and `mysql_common` `=0.37.3`, both MIT OR Apache-2.0, checked on crates.io on 2026-09-30 | the workspace pins both; `mysql_common` 0.38 is a separate upgrade | crates.io API |
| `mysql_common` 0.37.3 panics on data: `Value::deserialize_bin` ends in `unimplemented!` for type codes 17, 18, 19, 20 and 243 (`src/value/mod.rs:449`); `FromRow` panics on a conversion failure (`src/row/convert/mod.rs:84`); `Value::as_sql` overflows on a hostile `Time` | the driver refuses those codes before the first row, reads through `Row` and raw values only, and never calls `as_sql` | source at 0.37.3 |
| `mysql_common`'s `ParsedNamedParams` rewrites `:name` into `?` in any statement text it is handed | the driver refuses a text the library would rewrite instead of sending something other than what the user wrote | source at 0.37.3 `src/named_params.rs` |
| `mysql_async` 0.37.1 `is_last_result_set_packet` indexes `packet[0]` (`src/queryable/mod.rs:47,49`) | an empty packet from the server panics inside the library: residual risk, not reachable from the driver | source at 0.37.1 |
| `mysql_async` 0.37.1: `Opts`, `OptsBuilder` and `Conn` derive `Debug` over fields that hold the password | no driver type derives `Debug` over them; hand-written `Debug` everywhere ([I-03](../CLAUDE.md#i-03)) | `src/opts/mod.rs`, `src/conn/mod.rs` |
| `mysql_async` 0.37.1 reads `MYSQL_ASYNC_BUFFER_POOL_CAP`, `MYSQL_ASYNC_BUFFER_SIZE_CAP` and `MYSQL_ASYNC_BUFFER_INIT_CAP` from the environment (`src/buffer_pool.rs:21-31`) | the only environment reads in the driver's dependency path; they size buffers, never behavior | source at 0.37.1 |
| `mysql_async` 0.37.1: `exec` given a `&str` prepares an implicit statement that is never closed when `stmt_cache_size` is `0`; `Conn::disconnect` and `Drop` drain pending results before closing | the driver prepares, executes and closes explicitly; a connection whose results would panic in the drain is leaked, not dropped | `src/queryable/mod.rs`, `src/conn/mod.rs` |

### Observed against the servers — 2026-09-30

A probe on `mysql_async` 0.37.1 (default features off, `minimal-rust`), run
against the official Docker images `mysql:8.4` (8.4.11), `mysql:9.7` (9.7.2),
`mariadb:11.8` (11.8.9) and `mariadb:12.3` (12.3.3). The two MariaDB versions
gave identical results.

| Probe | MySQL 8.4 / 9.7 | MariaDB 11.8 / 12.3 |
|---|---|---|
| Prepare `SELECT 1; DROP TABLE b`, `USE t; DROP TABLE b`, `START TRANSACTION; DROP TABLE b`, `CREATE TRIGGER … SET @z = 1; DROP TABLE b`, `SELECT 1 /*!; DROP TABLE b */` | 1064 each; `b` still exists | same |
| Prepare `CREATE TRIGGER`, `CREATE PROCEDURE … BEGIN …; END`, `USE`, `START TRANSACTION`, `BEGIN`, `SAVEPOINT`, `LOCK TABLES`, `ALTER VIEW`, `LOAD DATA` | 1295 | prepared |
| Prepare `SET @x = 1`, `CREATE VIEW`, `COMMIT`, `EXPLAIN SELECT` | prepared | prepared |
| `SELECT 1; SELECT 2` through `COM_QUERY` | two result sets | two result sets |
| `SET SESSION TRANSACTION READ ONLY`, then `INSERT`, `UPDATE`, `DELETE`, `CREATE TABLE`, `DROP TABLE`, `TRUNCATE`, `ALTER TABLE`, `RENAME TABLE`, `CREATE INDEX`, `CREATE TEMPORARY TABLE`, text or prepared | 1792 (25006) each; `SELECT` runs | same |
| `SERVER_STATUS_IN_TRANS` in `last_ok_packet()` after `START TRANSACTION` | set | set |
| `VECTOR(3)` | 8.4: syntax error; 9.7: type 242, charset 63, length 12, `decimals` 31, bytes `00 00 80 3f …` | `VAR_STRING` (253), charset 63, length 12 |
| `JSON` | type 245, charset 63, flags `BLOB` and `BINARY` | `BLOB` (252), charset 224 (`utf8mb4`), flags `BLOB` and `BINARY` |
| `POINT` | type 255, 4-byte SRID then WKB | same |
| `DECIMAL(10,2)`, `(65,30)`, `(10,0) UNSIGNED`, `(5,5)`: length / `decimals` | 12/2, 67/30, 10/0, 7/5; values as decimal text | same |
| `TINYINT(1)` holding 100 | `Int(100)`, length 1 | same |
| `BIT(12)` holding `b'101'` | 2 bytes, big-endian | same |
| `0000-00-00`, `2024-00-15` in `DATE`/`DATETIME`/`TIMESTAMP` under `sql_mode = ''`; `TIME '-838:59:59'` | decoded as `Value::Date` / `Value::Time`, no error | same |
| `KILL QUERY` from a second connection of an account granted only `SELECT` | `OK`; the victim gets 1317 (70100); its connection runs `SELECT 7` next. An interrupted `SLEEP()` returns 1 instead of failing | same |
| `LOAD DATA LOCAL INFILE '/etc/hosts'` with `local_infile = 1` and no handler | client error "Handler is not specified"; 0 rows; the connection is closed | same |

The driver's integration tests (`drivers/oxyn-driver-mysql/src/integration.rs`),
run on 2026-09-30 against `mysql:8.4` (8.4.11), `mysql:9.7` (9.7.2) and
`mariadb:11.8` (11.8.9), observed in addition:

| Observation | MySQL 8.4 / 9.7 | MariaDB 11.8 |
|---|---|---|
| `START TRANSACTION`; `INSERT` a row; a long `SELECT` interrupted by `KILL QUERY`; `COMMIT` on the same connection | the `SELECT` fails with 1317; `SERVER_STATUS_IN_TRANS` stays set; after `COMMIT` the row is there: `KILL QUERY` leaves the transaction open | same |
| `KILL QUERY` sent before the victim's `COM_STMT_EXECUTE` is written | the kill finds nothing to interrupt, and the statement then runs to its end (8.4.11) | not probed |
| Prepare `CALL p()` for a procedure that returns a result set | 0 columns at prepare; the columns arrive with the execute | same |

### Prepare metadata and transaction status — observed on 2026-10-01

**A `mysql_async` 0.37.1 defect.** Every packet read outside a pending result
goes through `Conn::handle_packet` (`src/conn/mod.rs:874`), which parses it as
an OK packet and, when that succeeds, stores it with `handle_ok`
(`src/conn/mod.rs:306`). A `COM_STMT_PREPARE_OK` starts with `0x00` like an OK
packet: it is stored as the last OK packet, with "status flags" read from the
statement id and column count bytes. Against MySQL 8.4.11, a prepare inside an
open transaction made `last_ok_packet()` say none was open. The driver keeps
the state observed before a successful prepare (`Shared::prepare`), and a
cursor cancelled before it sent its statement does not read that packet as a
status.

**`mysql_async` rewrites a statement's columns after its execution.**
`Statement::columns()` read after `exec_iter` returns the execution's columns
(`update_columns_metadata`, `src/queryable/stmt.rs:235`): the prepare's must be
read before executing to compare them.

The prepare's columns against the execution's, through `Conn::prep` then
`exec_iter`, decoded into the driver's types, on `mysql:8.4` (8.4.11),
`mysql:9.7` (9.7.2) and `mariadb:11.8` (11.8.9):

| Statement | MySQL 8.4 / 9.7 | MariaDB 11.8 |
|---|---|---|
| `SHOW PROCESSLIST`, `SHOW FULL PROCESSLIST`, `SHOW ENGINES`, `SHOW PLUGINS`, `SHOW PRIVILEGES`, `SHOW ENGINE INNODB STATUS`, `SHOW PROFILES` | 0 columns at prepare; 8 (processlist), 6, 5, 3, 3, 3 at execution | same, 9 for the process list |
| `OPTIMIZE`, `ANALYZE`, `REPAIR`, `CHECKSUM TABLE`, `EXPLAIN SELECT`, `EXPLAIN FORMAT=JSON`, `SHOW CREATE TABLE`, `SHOW CREATE DATABASE`, `SHOW GRANTS`, `SHOW BINARY LOG STATUS` | 0 columns at prepare; the columns at execution (`EXPLAIN`: 12 on 8.4, 1 on 9.7) | identical at prepare and execution |
| `CHECK TABLE`, `SHOW WARNINGS`, `SHOW ERRORS` | 1295 at prepare: the text protocol runs them | `CHECK TABLE` identical at prepare and execution; `SHOW WARNINGS`, `SHOW ERRORS`: 0 columns at prepare, 3 at execution |
| `SHOW INDEX` | `Seq_in_index`: `Int64` at prepare, `Int32` at execution | identical at prepare and execution |
| `SHOW TABLE STATUS` | `Version`: `Int64` at prepare, `Int32` at execution | identical at prepare and execution |
| `SELECT ? AS x` executed with an integer | `Text` at prepare, `Int64` at execution | `Binary` at prepare, `Int64` at execution; also `SELECT ? + 1` (`Float64`) and `COALESCE(?, 1)` (`Int32`) |
| `SHOW STATUS`, `SHOW VARIABLES`, `SHOW COLLATION`, `SHOW OPEN TABLES`, `SHOW EVENTS`, `SELECT 1` | identical at prepare and execution | identical |

The driver therefore learns every result's schema from its execution, never
from the prepare. `execute` returns once the server answered the execution,
and the moment differs per server: a `COUNT(*)` over a ten-way cross join of
ten rows answered only when `KILL QUERY` ended it, after 4 s, on MySQL 8.4 and
9.7, as did a streaming ten-way join; a seven-way join of 200-byte rows
answered in 45 ms. On MariaDB the streaming ten-way join answered in 13 ms.
Until then, the execution is stopped through its token.

**A failing statement keeps the transaction.** In an open transaction, an
`INSERT` refused with 1062 at execution and a `SELECT` refused with 1146 at
prepare leave `SERVER_STATUS_IN_TRANS` set in the next `COM_PING` OK packet,
and the row inserted before them is there after `COMMIT`, on all three
servers. `mysql_async` forgets the last OK packet on an error packet
(`handle_err`, `src/conn/mod.rs:313`): the driver asks again with `COM_PING`.


## COPY destinations and classification — checked 2026-10-03

The registry source of `sqlparser` **0.62.0**, pinned in `Cargo.lock`, defines
`CopyTarget::{Stdin, Stdout, File { filename }, Program { command }}` in
`src/ast/mod.rs`; `Statement::Copy` carries both `to` and `target`.
Only a `TO STDOUT` destination exports to the client without a server-side
file or program effect. The classification treats file exports and program
execution as writes with dedicated approval reasons; rejected COPY syntax
remains `Unknown`, hence mutating.

Sources: the installed crates.io registry source, and PostgreSQL **18**
[`COPY`](https://www.postgresql.org/docs/18/sql-copy.html), checked on
2026-10-03. PostgreSQL executes `PROGRAM` on the database server and resolves
file paths there; `STDOUT` transfers data through the client connection.
No live database was used for this verification.
