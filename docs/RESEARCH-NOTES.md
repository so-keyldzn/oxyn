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

## PostgreSQL-compatible session reset — check of 2026-10-22

Amazon Redshift documents `RESET ALL` as resetting all runtime parameters,
including session context variables. Its [`SET SESSION AUTHORIZATION` page](https://docs.aws.amazon.com/redshift/latest/dg/r_SET_SESSION_AUTHORIZATION.html)
documents `SET ... DEFAULT` but requires a database superuser for the command;
it does not document `RESET SESSION AUTHORIZATION`. The PostgreSQL driver's
Redshift flavor therefore sends only the documented unprivileged reset,
`RESET ALL`, after a writable execution; PostgreSQL keeps its fuller reset
sequence. Source: [Amazon Redshift RESET](https://docs.aws.amazon.com/redshift/latest/dg/r_RESET.html),
read on 2026-10-22.


## AI transport confinement — 2026-10-03

The pinned reqwest **0.13.4** and its checksum were verified against the
[crates.io sparse registry](https://index.crates.io/re/qw/reqwest) and `Cargo.lock`.
Its [ClientBuilder API](https://docs.rs/reqwest/0.13.4/reqwest/struct.ClientBuilder.html)
provides `no_proxy` (including automatic system proxies), `dns_resolver`, and
`resolve_to_addrs`. The [Resolve contract](https://docs.rs/reqwest/0.13.4/reqwest/dns/trait.Resolve.html)
returns a future of socket addresses; an explicit URL port overrides their
ports. Oxyn uses a resolver that rejects a non-loopback answer for a measured
`Local` destination, and disables proxies for every provider family. Literal
IPs bypass DNS and are checked before building a local client.

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

Re-read on 2026-10-20, at `tauri` 2.12.1, `tauri-codegen` 2.7.1 and
`tauri-utils` 2.10.1 (`Cargo.lock`), three more facts still hold, to re-check
at every bump of `tauri`; the rest of this file keeps the versions printed in
its own cells:

- the `Emitter` fact: `tauri` 2.12.1 `src/lib.rs`, `pub trait Emitter` at line
  961, still exactly six emission methods (`emit` 974, `emit_str` 981,
  `emit_to` 1009, `emit_str_to` 1020, `emit_filter` 1047, `emit_str_filter`
  1058), which `clippy.toml` forbids
  ([ADR-0043](adr/0043-multi-fenetre.md)); used in `clippy.toml` and in
  [SECURITY](SECURITY.md#input-surface);
- the window-label caveat: `tauri` 2.12.1 `src/window/plugin.rs`, `get_window`
  (lines 13-21) looks the window up by the label the caller sends and falls
  back to the calling window only for no label or an empty one;
  `start_dragging` (`setter!`, line 165) and `internal_toggle_maximize` (lines
  220-233) both go through it; used in
  [SECURITY](SECURITY.md#input-surface);
- the style nonce: `tauri-codegen` 2.7.1 `src/context.rs` imports
  `inject_nonce_token` from `tauri_utils::html2` (line 28) and calls it on each
  parsed HTML asset (line 80; a second call, on the isolation page, at line
  117); `tauri-utils` 2.10.1 `src/html2.rs`: `inject_nonce_token` (lines 67-77)
  selects the `style` elements of the parsed static document (line 75) and
  `inject_nonce` skips those that already carry a nonce (lines 51-60); at
  run time, `tauri` 2.12.1 `src/manager/mod.rs` calls `replace_csp_nonce`
  (the `style-src` call at line 97, the `script-src` one at line 87; defined
  at 126-153), which adds `'nonce-…'` sources to
  `style-src` only when the served asset held the token or style hashes exist
  (line 140). Only the `style` elements of the static HTML are nonced: a
  `<style>` inserted into the DOM at run time is not. `src/html.rs` of
  `tauri-utils` (feature `html-manipulation`) carries the same logic but is
  not the module `tauri-codegen` uses. What the static HTML of Oxyn's build
  holds is a property of that build, not of Tauri: it stays dated where
  [SECURITY](SECURITY.md#input-surface) states it.

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
| `storybook` · `@storybook/react-vite` · addons · `eslint-plugin-storybook` | `10.6.2` | `10.6.3` | read on 2026-10-21 (`pnpm view`): `10.6.2`, published on 2026-10-12, is the first patched version for [GHSA-f2ph-cmg5-wxc8](https://github.com/advisories/GHSA-f2ph-cmg5-wxc8) (critical, development only); `10.6.3` was published on 2026-10-20 |
| `vitest` · `@vitest/browser-playwright` | `4.1.11` | `5.0.1` | `@storybook/addon-vitest@10.6.0` required `vitest ^3 \|\| ^4`; `10.6.2` also accepts `^5` (read on 2026-10-21), the move to Vitest 5 is not made yet |
| `playwright` | `1.63.0` | same | Chromium only, for the stories |
| `@uiw/react-codemirror` · `@codemirror/lang-sql` | `4.25.11` · `6.10.0` | same | — |
| `@codemirror/lang-json` | `6.0.2` | same | read on 2026-10-23 (published on 2025-06-19, MIT); the grammar of the MongoDB console, whose command is one Extended JSON document ([ADR-0063](adr/0063-mongodb-driver-database-commands-in-extended-json.md)). Brings `@lezer/json` 1.0.3 (published on 2024-12-29, MIT) |
| `zod` | `4.6.7` | same | read on 2026-09-16 (`4.6.5`); validates IPC responses ([ADR-0031](adr/0031-validation-des-reponses-ipc.md)). Had been declared since ADR-0029 and imported nowhere. Moved to `4.6.7` on 2026-10-23: `4.6.5` is vulnerable to [GHSA-mrhx-c7xc-m3w5](https://github.com/advisories/GHSA-mrhx-c7xc-m3w5), see the override below |
| `lexical` · `@lexical/react` | `0.51.0` | same | read on 2026-09-24 (`npm view`, dist-tag `latest`, published on 2026-09-23); the assistant's question field and its `@` mentions. `@lexical/react` declares `yjs` as a peer: it only serves the collaboration plugin, not imported |

| `@xyflow/react` | `12.11.6` | same | read on 2026-09-24 (published on 2026-09-01); the table diagram. Peers `react >=17` |
| `@dagrejs/dagre` | `3.1.1` | same | read on 2026-09-24 (published on 2026-08-08); layout of the diagram. **Not** `dagre` 0.8, abandoned, nor `elkjs` |
| `shiki` | `4.4.3` | same | read on 2026-09-24 (published on 2026-08-10); highlighting of code in responses, with the **JavaScript engine** (`shiki/engine/javascript`) and tokens, never `codeToHtml` |
| `mermaid` | `11.17.2` | `12.0.0` | read on 2026-09-24 (11.17.2 published on 2026-08-25, 12.0.0 on 2026-09-10). **12 targets Safari 17.4+ and ES2024**, whereas the target is macOS 13.0 (`minimumSystemVersion`), shipped with Safari 16: we stay on the latest 11.x. To reopen when the minimum target guarantees a WebKit 17.4 |

### npm security overrides — checked on 2026-10-06

Versions were read from the npm registry with `pnpm view <package> versions
--json`; dependency ranges and licenses with `pnpm view <package>@<version>`.

| Package | Pinned version | Reason and source |
|---|---|---|
| `source-map-js` | `1.2.2` | First patched version for [GHSA-68fv-2mgg-jv7q](https://github.com/advisories/GHSA-68fv-2mgg-jv7q); BSD-3-Clause. [Release notes](https://github.com/7rulnik/source-map-js/releases/tag/v1.2.2) confirm the indexed source-map denial-of-service fix. Even Vite `8.3.3` permits PostCSS `8.5.28`, whose `source-map-js` range is `^1.2.1`; a Vite bump alone does not require the fix. |
| `katex` | `0.18.2` | First patched version for [GHSA-238p-pmpm-9mq7](https://github.com/advisories/GHSA-238p-pmpm-9mq7); MIT. [Release notes](https://github.com/KaTeX/KaTeX/releases/tag/v0.18.2) confirm the settings prototype-pollution fix. Mermaid `11.17.2` remains the latest 11.x; even `12.1.0` still requires `katex ^0.16.47`, excluding the fix. Keep Mermaid's existing browser target and override KaTeX. |
| `seroval`, `seroval-plugins` | `1.6.8` | **Shipped in the production bundle**: `@tanstack/react-router` > `@tanstack/router-core` (`seroval ^1.6.2`, `seroval-plugins ^1.6.2`) serializes the router state, and its `seroval/plugins/web/ReadableStream` code is in `dist/client/assets/index-*.js` (checked on 2026-10-07 after `pnpm build`). The development path `@tanstack/react-devtools` > `@tanstack/devtools` > `solid-js` is the one that needs the override; the devtools themselves are absent from the production bundle since 2026-10-07 (loaded only under `import.meta.env.DEV`). Covers [GHSA-p6vx-979v-rg4c](https://github.com/advisories/GHSA-p6vx-979v-rg4c) (patched in `1.6.2`), [GHSA-jp82-f5mq-hwhp](https://github.com/advisories/GHSA-jp82-f5mq-hwhp) and [GHSA-mv8w-475r-vwqw](https://github.com/advisories/GHSA-mv8w-475r-vwqw); MIT; `1.6.8` published on 2026-09-29. Added on 2026-10-06: `solid-js 1.9.15` pins `seroval ~1.5.4`, and the `solid-js 1.9.16` that requires `~1.6.8` was published that day at 19:11 UTC, which pnpm 11's minimum release age still refuses — the override avoids bypassing that safeguard. `seroval-plugins 1.6.8` peers on `seroval ^1.0`. Remove it once the lockfile resolves `solid-js >= 1.9.16`. |
| `zod@^4` | `4.6.7` | Added on 2026-10-23. First patched version is `4.6.6` for [GHSA-mrhx-c7xc-m3w5](https://github.com/advisories/GHSA-mrhx-c7xc-m3w5) (CVE-2023-54404, high, published on 2026-10-01): validating a large array with an array schema that has no length constraint accumulates one issue per failing element, with no cap, and exhausts memory in `zod <= 4.6.5`. Read from the npm registry on 2026-10-23: `4.6.6` published on 2026-10-11, `4.6.7` on 2026-10-12 (the latest, and accepted by pnpm 11's minimum release age); MIT. The application's own `zod` (IPC response validation, [ADR-0031](adr/0031-validation-des-reponses-ipc.md)) is pinned to `4.6.7` in `package.json`; the override is for the 17 paths of the audit through `@tanstack/react-start` > `@tanstack/start-plugin-core` > `@tanstack/router-generator` and `router-plugin`, which declare `zod ^4.5.4` and kept `4.6.5` in the lockfile. Scoped to `^4`: `shadcn` > `@modelcontextprotocol/sdk` needs `zod 3.25.76`, which the advisory does not cover and which must not be moved. `make audit-npm` (`pnpm audit --prod`) reports no known vulnerability with it. Remove it once the TanStack packages resolve `zod >= 4.6.6` without it. |

pnpm `11.1.2` ignores `package.json#pnpm`: overrides belong in
`apps/desktop/pnpm-workspace.yaml`, as documented in the
[pnpm migration guide](https://github.com/pnpm/pnpm.io/blob/main/docs/migration.md)
and confirmed by a lockfile-only install. Remove each override when the parent
dependency requires a patched version. No other dependency is upgraded.

### npm advisories fixed by a lockfile refresh — checked on 2026-10-07

Refreshed by #207 on 2026-10-06; recorded on 2026-10-07. Advisories read from
the GitHub Advisory Database (`gh api /advisories?ecosystem=npm&affects=<package>@<old version>`),
versions and publication dates from the npm registry (`npm view <package> time`),
ranges from the installed parents' `package.json`. Each parent's range already
admits the patched version: **lockfile refresh, no override**, nothing to remove
later. All four are development-only (shadcn CLI, TanStack devtools Vite plugin).

| Package | Version | Advisory | First patched | Parent and the range that admits it | Source |
|---|---|---|---|---|---|
| `@modelcontextprotocol/sdk` | `1.30.0` → `1.32.1` | [GHSA-6qxp-vccf-f47h](https://github.com/advisories/GHSA-6qxp-vccf-f47h) (high, `>= 1.12.0, < 1.31.0`) | `1.31.0` | `shadcn 4.21.0`: `^1.26.0` | `1.32.1` published on 2026-10-05; MIT |
| `proxy-addr` | `2.0.7` → `2.0.8` | [GHSA-jqcg-44mw-7w3h](https://github.com/advisories/GHSA-jqcg-44mw-7w3h) (critical, `>= 1.1.0, < 2.0.8`) | `2.0.8` | `express 5.2.1` (under `@modelcontextprotocol/sdk`): `^2.0.7` | `2.0.8` published on 2026-09-15; MIT |
| `shell-quote` | `1.10.0` → `1.12.0` | [GHSA-pqg4-j6r4-53mv](https://github.com/advisories/GHSA-pqg4-j6r4-53mv) (critical, `>= 1.8.4, < 1.11.0`) | `1.11.0` | `launch-editor 2.14.1` (under `@tanstack/devtools-vite`): `^1.8.4` | `1.12.0` published on 2026-10-02; MIT |
| `fast-uri` | `3.1.7` → `3.1.8` | [GHSA-hrr3-gc8f-f4qj](https://github.com/advisories/GHSA-hrr3-gc8f-f4qj) (medium, `>= 3.0.0, < 3.1.8`) | `3.1.8` | `ajv 8.20.0` (under `@modelcontextprotocol/sdk`, `conf`): `^3.0.1` | `3.1.8` published on 2026-09-15; BSD-3-Clause |

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
| `sqlparser` logs through the `log` facade at `debug`: the whole statement on `sqlparser::parser` (`Parser::try_with_sql`) and the tokens it reads, string literals included, on `sqlparser::parser` and `sqlparser::dialect::*`; its `ParserError` quotes the token it stopped at | sources of `sqlparser@0.62.0`, `src/parser/mod.rs`, `src/dialect/postgresql.rs`, and observed, `logging::tests` | 2026-10-04 |
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
| `actions/upload-artifact` | `v7.0.2`, `cf430e030ddbb5b0abf93d22962f4752f3646cd9`, published on 2026-10-07 (the latest release). Inputs used by `livraison.yml`: `name`, `path`, `if-no-files-found: error`, `compression-level` (`0`: no compression, which the action recommends for files that do not compress); `overwrite` defaults to `false`, which fails an upload whose name already exists in the run | GitHub API, `action.yml` at that SHA | 2026-10-22 |
| `actions/download-artifact` | `v8.0.2`, `9000827ccba6bdab643e8b6fd33ac0654aef8333`, published on 2026-10-07 (the latest release). Without `github-token`, it downloads from the current repository and run; `name` selects one artifact; `digest-mismatch` defaults to `error` | GitHub API, `action.yml` at that SHA | 2026-10-22 |
| Tauri CLI release binary for the `signature` job: GitHub release `tauri-cli-v2.12.1`, published on 2026-09-30 from tauri commit `30da1fd6e17de6107ecc850c95dfb16b5729f2dd` — the commit of `tauri-cli` 2.12.1 read in the [updater contracts](#tauri-updater-contracts--checked-on-2026-10-02). `cargo-tauri-x86_64-unknown-linux-gnu.tgz`: SHA-256 `04ef3a9a2ed7b7dc479bef08ab40deee80065c99fc778f79ebce775eb72bbeff`, equal to the digest GitHub lists for the asset; it holds `cargo-tauri` at its root, beside a README and two licenses. The binary is dynamically linked against `libc.so.6` (symbol versions up to `GLIBC_2.34`), `libm.so.6`, `libgcc_s.so.1` and `liblzma.so.5`. Run as `cargo-tauri signer sign`, it runs the subcommand: `main.rs` drops a leading `tauri` argument only when cargo passes one | [release tauri-cli-v2.12.1](https://github.com/tauri-apps/tauri/releases/tag/tauri-cli-v2.12.1), archive downloaded and hashed with `shasum -a 256`; [main.rs](https://github.com/tauri-apps/tauri/blob/30da1fd6e17de6107ecc850c95dfb16b5729f2dd/crates/tauri-cli/src/main.rs); `file` and `strings` on the extracted binary | 2026-10-22 |
| The macOS binary of the same release (`cargo-tauri-aarch64-apple-darwin.zip`, SHA-256 `ee35ee364a0c626df7852d9a5bc0c5394b6440772e9864cfb9b11dcb8d33f527`) prints `tauri-cli 2.12.1`. `script/livraison signer` run with it as `$TAURI` and a throwaway key from `cargo-tauri signer generate` wrote a `.sig` whose trusted comment is `timestamp:…\tfile:…\tversion:0.0.10`; `minisign -V` 0.12, from the macOS archive of its release, accepted it and refused the archive once one byte was appended. The Linux binary was not run: no Linux machine was used | run by hand, throwaway key and file | 2026-10-22 |
| Node required by the front end | vite 8.3.0: `^20.19.0 \|\| >=22.12.0`; vitest 4.1.11: `^20.0.0 \|\| ^22.0.0 \|\| >=24.0.0`; CI takes the 22 line, the dev machine's (22.23.2) | `engines` field of the installed packages | 2026-09-24 |
| Tauri system libraries on Debian/Ubuntu | `libwebkit2gtk-4.1-dev build-essential curl wget file libxdo-dev libssl-dev libayatana-appindicator3-dev librsvg2-dev` | `tauri-apps/tauri-docs`, branch `v2`, `src/content/docs/start/prerequisites.mdx`, commit `2e513e3` of 2026-08-20 | 2026-09-24 |
| Tauri system dependency on macOS for desktop-only development | Xcode, or the Xcode Command Line Tools (`xcode-select --install`) when not targeting iOS | `tauri-apps/tauri-docs`, branch `v2`, `src/content/docs/start/prerequisites.mdx`, commit `2e513e3` of 2026-08-20 (section "macOS") | 2026-10-22 |
| GitHub-hosted runner `ubuntu-24.04-arm`: standard runner, listed for public and private repositories, 4 CPUs and 16 GB of RAM on a public repository; "Use of the standard GitHub-hosted runners is free and unlimited on public repositories". `RUNNER_ARCH` is `ARM64` there. `livraison.yml` builds the ARM64 Linux packages natively on it — no cross toolchain, no multiarch WebKitGTK sysroot, no QEMU — and the Tauri bundler's AppImage tooling runs as on x86_64 | [GitHub-hosted runners reference](https://docs.github.com/en/actions/reference/runners/github-hosted-runners); proven by run `37246283509` of the temporary `essai-linux-arm64` workflow (PR branch `linux-arm64-release`) | 2026-10-04 |
| `keyring` 4.2.0 on Linux goes through `zbus-secret-service-keyring-store` and `secret-service` 5.2.0: pure Rust, without `libdbus` | `cargo tree --target x86_64-unknown-linux-gnu` | 2026-09-24 |
| `cargo-nextest` CI binary | `0.9.146`; `universal-apple-darwin` SHA-256 `39785160b3c2f6ed9a765049cf4fa79f3b39aa02eb7598a5a0e2a1a0b9ffb9a8`; `x86_64-unknown-linux-gnu` SHA-256 `682c21b777c333e96fd532e114d3a5a894e0729ab88d94c0a9f20f8419695428` | [nextest release cargo-nextest-0.9.146](https://github.com/nextest-rs/nextest/releases/tag/cargo-nextest-0.9.146) and its `.sha256` assets | 2026-10-03 |
| Server integration CI images | Docker Official Images `postgres:17` and `mysql:8.4`, the major/minor tags used by the PostgreSQL and MySQL integration-test headers. On this check, the tags resolve to PostgreSQL 17.11 and MySQL 8.4.11; the production-read companion starts the same PostgreSQL tag with an ephemeral, job-local TLS certificate. | [PostgreSQL Docker Hub tags](https://hub.docker.com/_/postgres/tags?name=17.&page=1); [MySQL Docker Hub tags](https://hub.docker.com/_/mysql/tags?name=8.&page=1) | 2026-10-22 |
| `cargo-deny` CI binary | `0.20.2`; `aarch64-apple-darwin` SHA-256 `fe67d82a10d8597a3549364cb733a3f9cc1bfff9031b7ae46384a9f2a72090c3`; `x86_64-unknown-linux-musl` SHA-256 `9f12ed4c49936e09b48bf862b595cde2fe64fcbd9d74dfacac6131ca824c8d5f` | [cargo-deny release 0.20.2](https://github.com/EmbarkStudios/cargo-deny/releases/tag/0.20.2) and its `.sha256` assets | 2026-10-03 |
| CI failed at every push since at least 2026-09-21, in about twenty seconds: pnpm missing from the runner, `make qualite` stopped before the front end | log of run `36034960589` | 2026-09-24 |
| None of the runs of the gate had saved a Cargo cache: all of them failed, and the step "Post Restaurer le cache Cargo" was `skipped`. Only the pnpm caches existed. On Linux, the stories took 275 s (1045 tests) before the failure | runs `36037137682` and `36036585873`, `GET /actions/caches` | 2026-09-24 |
| Branch protection is refused on this repository: "Upgrade to GitHub Pro or make this repository public" (403). Nothing therefore prevents merging a PR whose CI fails | `GET /repos/so-keyldzn/oxyn/branches/main/protection` | 2026-09-24 |
| Still refused, branch protection as well as rulesets (403, same message): the aggregate `qualite` job of ADR-0045 is not yet required by anything | `GET /repos/so-keyldzn/oxyn/branches/main/protection`, `GET /repos/so-keyldzn/oxyn/rulesets` | 2026-09-25 |
| The public repository's active ruleset `24429281` (`main requires qualite`) now requires pull requests (zero approvals) and the `qualite` check from GitHub Actions app `15368` on `main`; it forbids deletion and non-fast-forward updates. Administrator bypass is `always` since 2026-10-04 (it was `pull_request`): administrators may push directly to `main`, everyone else needs a pull request | `GET /repos/so-keyldzn/oxyn/rulesets/24429281`, `GET /repos/so-keyldzn/oxyn/rules/branches/main` | 2026-10-04 |
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
| Static manifest: `version` (a leading `v` is trimmed), `notes`, `pub_date`, and `platforms` mapping a key to `url` and `signature`. Without a target set by the application, the key looked up is `{os}-{arch}-{installer}` then `{os}-{arch}`; `os` is `darwin` on macOS, `installer` is `app` for a `.app` or DMG, `appimage`, `deb`, `rpm` | `latest.json` carries `darwin-aarch64`, `linux-x86_64-appimage` and, since 2026-10-04, `linux-aarch64-appimage` (`updater_arch` maps `target_arch` `aarch64` to `aarch64`); a bare `linux-x86_64` or `linux-aarch64` would also match a deb or rpm install | [updater.rs](https://github.com/tauri-apps/plugins-workspace/blob/e51128438011755f9e7277bad29b8c0978cf281c/plugins/updater/src/updater.rs), `RemoteRelease`, `get_urls`, `Installer::name`, `updater_os` |
| An endpoint whose scheme is not `https` is refused with `InsecureTransportProtocol` in a release build (a warning in debug), unless `dangerousInsecureTransportProtocol`. The check covers the endpoints only, not the `url` inside the manifest | The endpoint is HTTPS by construction; Oxyn refuses itself an archive URL that is not `https://github.com/…`; the archive's integrity rests on its signature | [config.rs](https://github.com/tauri-apps/plugins-workspace/blob/e51128438011755f9e7277bad29b8c0978cf281c/plugins/updater/src/config.rs), `validate_endpoints` |
| `UpdaterBuilder::timeout` applies to the check's request; the `Update` that `check` returns carries `timeout: None`, so `download` runs with no timeout unless `Update::timeout` is set | Oxyn's 30 s bound covers the check only; the download is ended by `cancel_update`, which aborts its task, or by the 60 s read timeout and HTTPS-only redirects set on the client through `UpdaterBuilder::configure_client` | updater.rs, `Updater::check` (the `Update { … timeout: None … }` literal), `Update::download` |
| `download` reads the whole archive into a `Vec<u8>`, then verifies the minisign signature against `pubkey` before returning it. The signature covers the archive's bytes and its trusted comment; the manifest is not signed | The bytes are kept in memory, already verified; whoever serves the manifest chooses which signed archive is offered | updater.rs, `Update::download`, `verify_signature` |
| `requireSignedVersion` (default `false`): after the signature check, the `version:` field of the trusted comment must equal the announced version, compared as semver; absent, the archive is refused (`MissingSignedVersion`), different, `SignedVersionMismatch` | Set to `true`: a forged manifest cannot pair a higher version with an older signed archive | config.rs, `require_signed_version`; updater.rs, `verify_signed_version` |
| The Tauri CLI writes `timestamp:…\tfile:…` in the trusted comment, and from 2.12.0 appends `\tversion:<app version>` when bundling (`sign_file(…, Some(settings.version_string()))`); 2.11.4 writes no version | The CLI is pinned to 2.12.1; an archive signed by an older CLI would be refused | [updater_signature.rs](https://github.com/tauri-apps/tauri/blob/30da1fd6e17de6107ecc850c95dfb16b5729f2dd/crates/tauri-cli/src/helpers/updater_signature.rs), `sign_file`; `crates/tauri-cli/src/bundle.rs` line 312 |
| `tauri signer generate -w <path>` prompts for a password when none is given, writes the private key there and the public key at `<path>.pub` | The runbook in [RELEASE](RELEASE.md#updater-signing-key) | `crates/tauri-cli/src/signer/generate.rs`, `helpers/updater_signature.rs`, `save_keypair` |
| `tauri signer sign` reads the key from `TAURI_SIGNING_PRIVATE_KEY` and its password from `TAURI_SIGNING_PRIVATE_KEY_PASSWORD`, writes `<file>.sig`, and with `--app-version <v>` calls the same `sign_file(…, Some(v))` as the bundler, hence the same `version:` field | Signing is a job of its own, `signature`, which builds nothing and never shares a job with the build (`script/livraison signer`) | `crates/tauri-cli/src/signer/sign.rs`, `app_version` |
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

## `ctutils` and `tokio` — security advisories of 2026-10-23

`cargo deny` (the last step of `make rust`) failed on every branch once two
advisories were issued on 2026-10-23 for crates that `Cargo.lock` already held.
Both were read on 2026-10-23 at [rustsec.org](https://rustsec.org/), and the
versions on crates.io the same day.

- [RUSTSEC-2026-0385](https://rustsec.org/advisories/RUSTSEC-2026-0385), type
  vulnerability, category `crypto-failure`, on `ctutils 0.4.2`, reached through
  `digest 0.11.3` by `hmac 0.13.0`, `md-5 0.11.0` and `sha2 0.11.0`. The
  mask-generation functions of `ctutils::Choice` are `const fn`, so they cannot
  reuse the `asm!`-backed constant-time code of the `cmov` crate: before 0.4.4
  the generated code could contain a branch on commonly used platforms, and its
  `u128` comparisons produced branches on 32-bit targets. Patched `>=0.4.4`.
- [RUSTSEC-2026-0386](https://rustsec.org/advisories/RUSTSEC-2026-0386), type
  `unsound`, categories `memory-corruption` and `use-after-free`, on
  `tokio 1.53.1`. The safe constructors `AsyncFd::new`, `with_interest`,
  `try_new` and `try_with_interest` accept any `T: AsRawFd`, so safe code can
  build an `AsyncFd` from a closed or reused file descriptor, and dropping it
  can free an allocation that the epoll or kqueue registration still points to.
  Patched `>=1.51.6, <1.52.0` and `>=1.53.3`; from those releases the four
  constructors are deprecated in favour of the unsafe `register` and
  `register_with_interest`.

**Real scope in Oxyn.** Neither crate is a direct dependency, and no Oxyn source
file calls `AsyncFd` (`git grep AsyncFd -- '*.rs'` finds nothing on 2026-10-23).
Transitive callers of the `tokio` constructors were not audited: the update is
taken because it is free, not because a call was found.

Fixed in `Cargo.lock` only, four lines: `ctutils` **0.4.2 → 0.4.5** (0.4.4 is
the first patched release, 0.4.5 the latest published on 2026-10-23) and `tokio`
**1.53.1 → 1.53.3** (published 2026-10-13, the smallest move that `^1.53.1` in
`Cargo.toml` allows, so no pinned manifest version moves). The checksums are the
ones cargo computed from the registry. Checked on 2026-10-23:
`cargo metadata --locked` needs no other change and
`cargo deny --all-features check` reports `advisories ok, bans ok, licenses ok,
sources ok`.

## `glib` 0.18 — advisory RUSTSEC-2024-0429, checked on 2026-10-20

`cargo deny` reports **RUSTSEC-2024-0429** (`informational = "unsound"`) on
`glib` 0.18.5, the only `glib` in `Cargo.lock`. `deny.toml` ignores it with a
written reason, which [SECURITY](SECURITY.md#dependencies) repeats. Every
fact below was read on 2026-10-20 and is **to re-read at every bump of
`tauri`, `gtk`, `wry`, `muda`, `tao` or `webkit2gtk`**.

- **The advisory**: file `crates/glib/RUSTSEC-2024-0429.md` of the RustSec
  advisory database, local checkout at HEAD
  `550efd3d587a29b2e2c2b21b17a440da4fede999` of 2026-10-08 (nothing fetched on
  2026-10-20). Published 2024-03-30, alias GHSA-wrw7-89jp-8q8g. Affected
  functions: `glib::VariantStrIter::next`, `nth`, `last`, `next_back` and
  `nth_back`, in `>=0.15.0,<0.20.0`; patched in `>=0.20.0`; unaffected below
  `0.15.0`. The mechanism: `impl_get` passes `&p`, an immutable reference to a
  null `*mut c_char`, as the out-argument of the variadic C function
  `g_variant_get_child`; the compiler may disregard that write, and
  `CStr::from_ptr` then receives NULL, which crashes on a NULL dereference. The
  fix passes `&mut p`.
- **Why nothing can be bumped**: `gtk` 0.18.2 (`Cargo.lock`) declares
  `glib = "0.18"` in its own `Cargo.toml` (`[dependencies.glib]`, registry
  source), which excludes 0.20.
- **Where it is compiled**: `cargo tree --offline -p oxyn-desktop --target
  <t> -i glib@0.18.5 -e normal` prints the same tree for
  `x86_64-unknown-linux-gnu` and `aarch64-unknown-linux-gnu`: `glib` ←
  `atk` 0.18.2 ← `gtk` 0.18.2 ← `muda` 0.20.0, `tao` 0.37.1, `tauri-runtime`
  2.12.1, `tauri-runtime-wry` 2.12.1, `webkit2gtk` 2.0.2, `wry` 0.57.0 and
  `tauri` 2.12.1. `cairo-rs`, `gdk`, `gdk-pixbuf`, `gdkx11`, `gio`,
  `javascriptcore-rs`, `pango` and `soup3` depend on `glib` directly as well.
  With `-e normal,build` on `aarch64-apple-darwin` and `x86_64-apple-darwin`,
  the same command prints "nothing to print": macOS does not compile it. The
  Linux packages that do ship it are the `paquets` job of
  `.github/workflows/livraison.yml` (commits `2a6f563` of 2026-09-24 for
  x86_64 and `a8039da` of 2026-10-04 for aarch64).
- **Source of `glib` 0.18.5** (registry): `src/variant_iter.rs` has
  `VariantStrIter` at line 102, its constructor `pub(crate) fn new` at line
  109, `impl_get` at line 118, `&p` at line 126 and `CStr::from_ptr` at line
  129. `src/variant.rs`: `Variant::array_iter_str` (lines 843-855, the
  construction at line 854) is the only function that builds a
  `VariantStrIter`; the type is re-exported at `src/lib.rs` line 40, and every
  other use sits under `#[cfg(test)]` (`variant.rs` from line 2191,
  `variant_iter.rs` from line 203).
- **Reachability**: `cargo tree --offline -p oxyn-desktop --target <t> -e
  normal,build --prefix none --format '{p}'` on the two Linux targets lists
  634 distinct external crate versions in their union (workspace crates
  excluded, dev-dependencies not examined). A recursive search for
  `array_iter_str` and `VariantStrIter` in the registry source of each of
  them matches `lib.rs`, `variant.rs` and `variant_iter.rs` of `glib` 0.18.5
  and nothing else; there is no match in `crates/`, `drivers/` or `apps/`
  either. No shipped code builds a `VariantStrIter`, so the five methods
  cannot be reached.
- **The gate**: `cargo deny --all-features check advisories --disable-fetch`
  prints `advisories ok` with the ignore entry. Run on 2026-10-20 with the
  `cargo-deny` 0.19.0 installed on the development machine; CI runs the
  version pinned in the "CI and GitHub delivery" section, which this run does
  not cover.

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
| `rustix` | **0.38.44** | Apache-2.0 WITH LLVM-exception OR Apache-2.0 OR MIT | killing the process **group** (feature `process`), and opening a user's agent file with `O_NOFOLLOW \| O_NONBLOCK` (feature `fs`, ADR-0049). The latest version is `1.1.4`; we align on `0.38.44`, the one `agent-client-protocol` already pulls, so as not to compile two copies of a system-call crate |

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
model that the call expired, and a new attempt on its part is refused while
the request still waits — no call of an agent runs while one of its requests is
before the user (`crates/oxyn-ai/src/external/mcp/turn.rs`) — or, once the
approval has taken it, asks for approval again, on screen: never a silent
replay ([I-13](../CLAUDE.md#i-13)).

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

## Editor completion — check of 2026-10-21

Read for [ADR-0057](adr/0057-editor-completion-reads-the-catalog-cache.md).
The PostgreSQL pages were fetched on 2026-10-21 and are those of
**PostgreSQL 18.6**, the current release that day.

| Fact | Source |
|---|---|
| `current_schemas(include_implicit boolean)` returns `name[]`: "all schemas presently in the effective search path, in their priority order". Entries of the `search_path` setting "that do not correspond to existing, searchable schemas are omitted". With `true`, implicitly searched system schemas such as `pg_catalog` are included. | [PostgreSQL 18: System Information Functions](https://www.postgresql.org/docs/18/functions-info.html) |
| Observed on a throwaway PostgreSQL 17.11 (Postgres.app, aarch64, `UTF8`) on 2026-10-22: `current_schemas(true)` answers `{pg_catalog,public}`, and `{pg_temp_1,pg_catalog,public}` once the session has created a temporary table — the temporary schema first, `pg_catalog` next. | `SELECT current_schemas(true)` run by hand on a server started with `initdb` for the occasion; the integration test of `oxyn-driver-postgres` asserts the first entry |
| Observed on the same server: an unquoted `USERS` reaches the table `users` and not `"Users"`, and an unquoted `É` does **not** reach `"é"` (`relation "É" does not exist`): the fold is to lowercase, on the ASCII letters. | `SELECT * FROM USERS` and `SELECT count(*) FROM É` run by hand on that server |
| The default `search_path` shows as `"$user", public`. A `$user` entry with no schema of that name "is ignored". `pg_catalog` "is always effectively part of the search path". "If there is no match in the search path, an error is reported, even if matching table names exist in other schemas in the database." | [PostgreSQL 18: Schemas, the schema search path](https://www.postgresql.org/docs/18/ddl-schemas.html#DDL-SCHEMAS-PATH) |
| `ORDER` and `GROUP` are "reserved, requires `AS`", `USER` and `TABLE` "reserved", in the PostgreSQL column. "Labeled "reserved" are those tokens that are not allowed as column or table names." | [PostgreSQL 18: SQL Key Words](https://www.postgresql.org/docs/18/sql-keywords-appendix.html) |
| `@codemirror/lang-sql` `6.10.0`, as installed in `apps/desktop/node_modules`: `nameCompletion` leaves a label bare when it matches `^[a-z_][a-z_\d]*$` (case-insensitively only if the dialect sets `caseInsensitiveIdentifiers`, default `false`), and otherwise sets `apply` to the first `identifierQuotes` character, the label and the closing character, **without doubling an embedded quote**. `identifierQuotes` defaults to `"` and is `` ` `` for MySQL. `SQLNamespace` accepts `{ self: Completion, children }`, so a completion can carry its own label and `apply`. | `dist/index.js` and `dist/index.d.ts` of the package |
| Same package, read again on 2026-10-22 for the editor's adapter: an explicit `apply` does **not** survive. When the cursor is inside a quoted identifier, `maybeQuoteCompletions` rewrites every label to the opening quote, the label and the closing quote — still without doubling — and sets `apply: undefined`. And `addNamespaceObject` splits every schema key on `.` (only an escaped `\.` keeps one), so a name holding a dot or a backslash is ambiguous as a key. A schema cannot carry the insertion text Rust composes: the front writes its own completion source over the snapshot, as the ADR's fallback says. `schemaCompletionSource`, `keywordCompletionSource` and `sql` are exported; `sourceContext` and `getAliases` are not, so the context of the cursor is read from the syntax tree again. | `dist/index.js` of the package (`maybeQuoteCompletions`, `addNamespaceObject`, `sourceContext`) |
| SQLite: "If the name of the table is unique across all attached databases and the main and temp databases, then the *schema-name* prefix is not required. If two or more tables in different databases have the same name and the *schema-name* prefix is not used on a table reference, then the table chosen is the one in the database that was least recently attached." The page does not say where `main` and `temp` rank in that tie. | [SQLite: ATTACH DATABASE](https://www.sqlite.org/lang_attach.html) |
| SQLite resolves an unqualified table name in `sqlite3FindTable` by looking in `temp` first (`aDb[1]`), "the main database is second" (`aDb[0]`), then "attached databases are in order of attachment" (`aDb[2…]`, the first match wins) — which is the ATTACH page's "least recently attached". Read at tag `version-3.50.2`, the version the driver bundles. | [`src/build.c`](https://github.com/sqlite/sqlite/blob/version-3.50.2/src/build.c), `sqlite3FindTable` |
| PostgreSQL: "The system uses no more than `NAMEDATALEN`-1 bytes of an identifier […] By default, `NAMEDATALEN` is 64 so the maximum identifier length is 63 bytes." | [PostgreSQL 18: Identifiers and Key Words](https://www.postgresql.org/docs/18/sql-syntax-lexical.html#SQL-SYNTAX-IDENTIFIERS) |
| MySQL 8.4: database, table and column names are at most 64 characters, and "identifiers are stored using Unicode (UTF-8)" — up to four bytes a character, hence 256 bytes at most. | [MySQL 8.4: Identifier Length Limits](https://dev.mysql.com/doc/refman/8.4/en/identifier-length.html) |
| Amazon Redshift's list of reserved words differs from PostgreSQL's: it includes `TIMESTAMP`, `TOP`, `OPEN`, `OFF`, `OLD`, `NEW`, `TAG`, `RAW`, `SYSTEM`, `PARTITION`, `PERCENT`, `IDENTITY`, `INTERVAL`, `SNAPSHOT`, `WITHOUT`, `NULLS` and `UNNEST`, none of which PostgreSQL 18.6 reserves as a table name (`VERSION` is on neither list). "You can use the reserved words with delimited identifiers (double quotation marks)." | [Amazon Redshift: Reserved words](https://docs.aws.amazon.com/redshift/latest/dg/r_pg_keywords.html), read on 2026-10-21 |
| Redshift does not support, among PostgreSQL's functions, "Array functions and operators", the "Set returning functions" `GENERATE_SERIES()` and `GENERATE_SUBSCRIPTS()`, and the "Schema visibility inquiry functions"; "some unsupported functions will not return an error when run on the leader node". Whether `unnest(…) WITH ORDINALITY` runs on a cluster was **not** tested: the driver refuses the resolution path on Redshift rather than ask. | [Amazon Redshift: Unsupported PostgreSQL functions](https://docs.aws.amazon.com/redshift/latest/dg/c_unsupported-postgresql-functions.html), read on 2026-10-21 |
| Unicode general categories `Cf` (format), `Zl` and `Zp` (line and paragraph separators): `00AD`, `0600–0605`, `061C`, `06DD`, `070F`, `0890–0891`, `08E2`, `180E`, `200B–200F`, `2028–202E`, `2060–2064`, `2066–206F`, `FEFF`, `FFF9–FFFB`, `110BD`, `110CD`, `13430–1343F`, `1BCA0–1BCA3`, `1D173–1D17A`, `E0001`, `E0020–E007F` — the list of `hides_text`, with `Cc` from the standard library and four fillers (`115F`, `1160`, `3164`, `FFA0`) that render blank. A character added to those categories by a later Unicode version is not caught until the list is refreshed. | Unicode 16.0.0 as shipped with Python 3.14.6 (`unicodedata.category` over every code point), 2026-10-21 |
| `JSON.parse` of a completion-shaped message (relations and columns with short names), Node `22.23.2` (V8): about 3 ms per MiB, measured on 2026-10-21 on the development machine with a throwaway script (`JSON.parse` of the serialized snapshot, median of repeated runs). The macOS webview runs JavaScriptCore, not V8: the figure is an order of magnitude, not the webview's. | measurement, recorded in [PERFORMANCE](PERFORMANCE.md) |

### Reserved words — `crates/oxyn-catalog/src/reserved.rs`

Extracted on 2026-10-21. Each list is the words an engine refuses as an
unquoted table or column name, and errs toward quoting.

| List | Source | Result |
|---|---|---|
| PostgreSQL | [`kwlist.h` at tag `REL_18_6`](https://github.com/postgres/postgres/blob/REL_18_6/src/include/parser/kwlist.h) | 494 keywords: 330 `UNRESERVED_KEYWORD`, 63 `COL_NAME_KEYWORD`, 23 `TYPE_FUNC_NAME_KEYWORD`, 78 `RESERVED_KEYWORD`. The 101 of the last two are kept: Appendix C reads them "reserved". |
| SQLite | [SQLite keywords](https://www.sqlite.org/lang_keywords.html) | 147 keywords, all kept. The page names no version. |
| MySQL | [`sql/lex.h`](https://github.com/mysql/mysql-server/blob/mysql-8.4.0/sql/lex.h) and [`sql/sql_yacc.yy`](https://github.com/mysql/mysql-server/blob/mysql-8.4.0/sql/sql_yacc.yy) at tag `mysql-8.4.0` | 731 words declared by `SYM` and `SYM_HK` (not the 36 `SYM_FN` function names, nor the `SYM_H` optimizer hints). 462 tokens appear in the rules `ident_keyword`, `ident_keywords_*` and `label_keyword`, which the parser takes as identifiers. The 264 words whose token appears in none are kept. |
| MariaDB | [`sql/lex.h`](https://github.com/MariaDB/server/blob/11.8/sql/lex.h) and [`sql/sql_yacc.yy`](https://github.com/MariaDB/server/blob/11.8/sql/sql_yacc.yy), branch `11.8` at commit `c20c21d7a1cd` | 690 words in `symbols[]`. 443 tokens belong to the closure of `keyword_ident` (`keyword_data_type`, `keyword_cast_type`, `keyword_set_special_case`, `keyword_sp_block_section`, `keyword_sp_head`, `keyword_sp_var_and_label`, `keyword_sp_var_not_label`, their `keyword_func_*` parts, `keyword_sysvar_type`, `keyword_verb_clause`, plus `FUNCTION_SYM`, `WINDOW_SYM`, `EXCEPTION_ORACLE_SYM`, `IGNORED_SYM`). The 247 words outside it are kept. |

The pages of the MySQL and MariaDB manuals that list reserved words are not the
source: `dev.mysql.com` answers a scripted request with a 403, and the archived
copy is not the list. The lists above are therefore **a superset of the
documentation's**: the grammar reserves what the parser cannot take as an
identifier, the manual says what a user may not. The extraction is an `awk` pass
per grammar (a word is reserved when its token is in no identifier rule),
checked on `select`, `table`, `order`, `group`, `key` and `index` (reserved) and
on `status`, `name`, `type`, `date`, `comment` and `count` (not). A new server
release is re-extracted the same way; the sizes are pinned by a test, so a hand
edit that drops a row fails. **The MySQL list is that of 8.4.0 and the MariaDB
list that of 11.8**: a word a later release reserves is inserted bare until the
lists are re-extracted, and a 9.x MySQL server has not been checked.

**Not checked.** Whether the MariaDB list equals the manual's page; how MySQL
and MariaDB resolve an unqualified name beyond the selected database; how
`lang-sql` resolves a qualified prefix and an alias with an explicit `apply` on
every completion — the spike at the start of the second pull request of the ADR.

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

### Context window overflow — reading of 2026-10-22

What a provider does with a request larger than the model's context window,
read for the bound on a remembered session (#271):

| Provider | What it documents | Source |
|---|---|---|
| Anthropic | "Everything in the request counts toward the context window": the system prompt, every message with its tool results, **the tool definitions**, and the output it generates. An input that alone exceeds the window gets a `400 invalid_request_error` ("prompt is too long") on every model; from Claude 4.5, an input that fits but leaves too little room stops the generation with `model_context_window_exceeded` | [Context windows](https://platform.claude.com/docs/en/build-with-claude/context-windows), § "How the context window works" and "Context window overflow behavior" |
| Anthropic | The error type is **not** specific: `invalid_request_error` is every `400` — "an issue with the format or content of your request", a spend limit, a refused prefill, a thinking setting. Errors have the shape `{"type":"error","error":{"type":…,"message":…}}`; `402` is `billing_error` | [Errors](https://platform.claude.com/docs/en/api/errors), § "HTTP errors" and "Error shapes" |
| OpenAI | **nothing**: the error-codes page lists no context-length code or type; a `400` is a request "malformed or missing some required parameters". A `context_length_exceeded` code circulates in forum reports only | [Error codes](https://developers.openai.com/api/docs/guides/error-codes) |
| Gemini | **nothing**: no code of the error table names an input over the window; `400 invalid_request` is "malformed or contains invalid parameters", `400 failed_precondition` a prerequisite such as billing, `402 payment_required` depleted Prepay credit, and the troubleshooting page says not to retry a `400`, `402` or `403` | [API errors](https://ai.google.dev/gemini-api/docs/api-errors), § "Standard API error codes"; [Troubleshooting](https://ai.google.dev/gemini-api/docs/troubleshooting) |

All four pages read on 2026-10-22.

Consequences in Oxyn. No provider documents a field of its own for the
overflow, so a status family cannot stand for it: a `400` or a `402` is just as
often a parameter or the credit, which a fresh context does not fix. Only
Anthropic's documented refusal becomes `OxynError::ContextTooLong` — a `400`
whose `error.type` is `invalid_request_error` **and** whose `error.message`
starts with "prompt is too long". The phrase is the documented one, read at the
start of the message only; if Anthropic rewords it, the refusal reads as an
ordinary one, which drops nothing. That refusal alone drops a remembered
session. **The gap that remains**: an overflow OpenAI, Gemini or an
OpenAI-compatible server refuses in its own words keeps the session, and the
same follow-up is refused again until the user starts a new conversation — the
failure says so.

The bound before sending is therefore the protection: a remembered session
may fill **half** of the window the model's listing declares
(`ModelInfo::context_window`), the tool definitions it offers counted with it,
since the window also holds the next question, its tool results and the
answer.

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
`.send()`. The exact schema of the response stream remains unverified; no value
is written for it here. The model list was checked on 2026-10-04: see
[Model listing](#model-listing--checked-on-2026-10-04).

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

## Model listing — checked on 2026-10-04

Read in each provider's official API reference on 2026-10-04, for
`oxyn_llm::list_models` and the provider form
([AI-PROVIDERS](AI-PROVIDERS.md#listing-a-providers-models)). Every provider
below lists its models; none documents listing without a key, except local
servers, which need none by default.

| Provider | Endpoint | Key | Shape | Pagination (documented maximum) | Context window | Display name |
|---|---|---|---|---|---|---|
| OpenAI | `GET https://api.openai.com/v1/models` | `Authorization: Bearer` | `{object:"list", data:[{id, created, owned_by}]}` | none | — | — |
| Anthropic | `GET https://api.anthropic.com/v1/models` | `x-api-key` + `anthropic-version` | `{data, first_id, last_id, has_more}` | `limit` (default 20, max **1000**), `after_id`, `before_id` | `max_input_tokens` | `display_name` |
| Gemini | `GET https://generativelanguage.googleapis.com/v1beta/models` | `x-goog-api-key` (or `?key=`, which Oxyn never uses) | `{models, nextPageToken}`; `name` is `models/<id>` | `pageSize` (default 50, max **1000**), `pageToken` | `inputTokenLimit` | `displayName` |
| Mistral | `GET https://api.mistral.ai/v1/models` | Bearer | `{object:"list", data}` | none | `max_context_length` | — |
| Groq | `GET https://api.groq.com/openai/v1/models` | Bearer | `{object:"list", data}` | none | `context_window` | — |
| OpenRouter | `GET https://openrouter.ai/api/v1/models` | Bearer, documented as required | `{data, total_count, links}` | `offset`, `limit` (default 500, max **1000**); the whole list when both are omitted | `context_length`, `top_provider.context_length` | `name` |
| Ollama | `GET http://localhost:11434/v1/models`; native `GET /api/tags` → `{models:[{name, model, details}]}` | none locally | OpenAI list | none | not returned by either | — |
| LM Studio | `GET http://localhost:1234/v1/models`; native `GET /api/v1/models` → `{models}` (v0 `/api/v0/models` superseded) | none unless the server's token setting is on | "OpenAI-compatible", not further documented | none | native only: `max_context_length` | native only: `display_name` |
| xAI | `GET https://api.x.ai/v1/models` (also `/v1/language-models` → `{models}`) | Bearer | `{object:"list", data}` | none | `context_length` (in the example only) | — |
| Together AI | `GET https://api.together.ai/v1/models` | Bearer | **bare array** | none | `context_length` | `display_name`; `type` among `chat`, `language`, `code`, `image`, `embedding`, `moderation`, `rerank` |
| Fireworks AI | `GET https://api.fireworks.ai/v1/accounts/{account_id}/models` — **not** under the OpenAI-compatible base `…/inference/v1` | Bearer | `{models, nextPageToken, totalSize}` | `pageSize` (default 50, max **200**), `pageToken` | `contextLength` | `displayName` |
| Cerebras | `GET https://api.cerebras.ai/v1/models` | Bearer | `{object:"list", data:[{id, created, owned_by}]}` | none | — | — |
| DeepSeek | `GET https://api.deepseek.com/models` (no `/v1`) | Bearer | `{object:"list", data}` | none | `context_window` | `name` |

Sources:
[OpenAI](https://developers.openai.com/api/reference/resources/models/methods/list),
[Anthropic](https://platform.claude.com/docs/en/api/models-list),
[Gemini](https://ai.google.dev/api/models),
[Mistral](https://docs.mistral.ai/api/endpoint/models),
[Groq](https://console.groq.com/docs/api-reference),
[OpenRouter](https://openrouter.ai/docs/api-reference/models/get-models),
[Ollama `/api/tags`](https://docs.ollama.com/api/tags) and
[OpenAI compatibility](https://docs.ollama.com/api/openai-compatibility),
[LM Studio REST](https://lmstudio.ai/docs/developer/rest/list),
[LM Studio OpenAI compatibility](https://lmstudio.ai/docs/developer/openai-compat/models),
[LM Studio authentication](https://lmstudio.ai/docs/developer/core/authentication),
[xAI](https://docs.x.ai/developers/rest-api-reference/inference/models),
[Together AI](https://docs.together.ai/reference/models-1),
[Fireworks](https://docs.fireworks.ai/api-reference/list-models) and
[its OpenAI compatibility](https://docs.fireworks.ai/tools-sdks/openai-compatibility),
[Cerebras](https://inference-docs.cerebras.ai/api-reference/models),
[DeepSeek list](https://api-docs.deepseek.com/api/list-models) and
[base URL](https://api-docs.deepseek.com/). All read on 2026-10-04.

What the code takes from it:

* **OpenAI-compatible family** — one request, no pagination: none of these
  providers documents one, and OpenRouter returns its whole list without
  `offset`/`limit`. The body is `{data:[…]}` or Together's bare array; the
  context window is read from `context_length`, `context_window` or
  `max_context_length`, the display name from `name` or `display_name`;
  Together's `image`, `embedding`, `moderation` and `rerank` entries are left
  out. Ollama and LM Studio are listed through `/v1/models`, the path their chat
  uses — never through their native endpoints, which a conversation does not
  use.
* **Anthropic** — `limit=1000`, cursor `after_id`, at most ten pages.
* **Gemini** — `pageSize=1000`, cursor `pageToken`, at most ten pages; the
  `models/` prefix is stripped, models without `generateContent` (embeddings)
  are left out. The key goes in `x-goog-api-key`.
* **Fireworks** cannot be listed through an OpenAI-compatible declaration: its
  list is per account, outside the chat base. The form reports `unsupported`.

Could not be verified: no maximum response size for the unpaginated
providers; the shape of LM Studio's `/v1/models` beyond "OpenAI-compatible";
the fields of Ollama's `/v1/models` beyond `created` and `owned_by`; xAI's host
and Bearer header are not written on its reference page; whether OpenRouter
actually answers without a key; whether Fireworks serves
`GET /inference/v1/models`.

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
source of the versions `Cargo.lock` pinned that day, unpacked in the local
registry: `tauri` 2.11.5, `tauri-utils` 2.9.3, `tauri-runtime-wry` 2.11.4, `wry` 0.55.1, `muda`
0.19.3; on the front-end side, `@codemirror/commands` 6.11.0, a transitive
dependency of `@uiw/react-codemirror`. The Emission row was re-read on
2026-10-20 at `tauri` 2.12.1; the other rows keep the versions printed in their
own cells. What tao does with `terminate:`
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
| Emission | `Emitter` carries six emission methods: `emit`, `emit_str`, `emit_to`, `emit_str_to`, `emit_filter`, `emit_str_filter` — forbidden by `clippy.toml` (ADR-0043) | `tauri` 2.12.1 `src/lib.rs`, trait `Emitter` (line 961), re-read on 2026-10-20 |
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
| Binary of `cargo-about` 0.9.2 for the ARM64 Linux release job | `aarch64-unknown-linux-musl`: SHA-256 `af5169282fb6f84e13471493f405437e43ac517744c9ae12fbe2cdf0a6f0e5a8`, equal to the published `.sha256`; the archive holds `cargo-about-0.9.2-aarch64-unknown-linux-musl/cargo-about` | [GitHub release](https://github.com/EmbarkStudios/cargo-about/releases/tag/0.9.2), archive downloaded and hashed with `shasum -a 256` | 2026-10-04 |
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

## MongoDB client library and server — checked on 2026-10-23

For [ADR-0063](adr/0063-mongodb-driver-database-commands-in-extended-json.md).

| Fact | Consequence | Source |
|---|---|---|
| Pinned: `mongodb` `=3.9.1` (2026-09-10, Apache-2.0, `rust-version` 1.88) and `bson` `3.1.0` (2025-11-20, MIT, `rust-version` 1.81), the latest stable releases | the workspace pins the client exactly; `bson` is the release `mongodb` resolves to | crates.io API |
| `mongodb` 3.9.1 features: `default` = `compat-3-0-0` (which pulls `bson-2`) + `rustls-tls` (rustls on `ring`) + `dns-resolver`; `rustls-tls-aws-lc` puts rustls and `tokio-rustls` on `aws_lc_rs`; `bson-3` selects `bson` 3; `dns-resolver` pulls `hickory-resolver` 0.26 for `mongodb+srv://`; compression, `aws-auth`, `gssapi-auth`, OIDC and `in-use-encryption` are separate features | `default-features = false` with `compat-3-3-0`, `bson-3`, `rustls-tls-aws-lc`, `dns-resolver`: the workspace's TLS provider, no unused authentication backend | `mongodb` 3.9.1 `Cargo.toml` |
| The rustls features trust `webpki-roots` (the Mozilla bundle compiled in), not the system store | a server signed by a private authority needs the form's CA file | `mongodb` 3.9.1 `Cargo.toml` (`dep:webpki-roots`) |
| `DRIVER_MIN_WIRE_VERSION = 9`, `DRIVER_MAX_WIRE_VERSION = 29`; the README states "MongoDB 4.4+" | servers older than 4.4 are refused by the library at the handshake | `src/sdam/description/server.rs`; `README.md` at 3.9.1 |
| `mongocrypt-sys` 0.1.6 (`links = "mongocrypt"`) appears in `Cargo.lock` as an optional dependency of `in-use-encryption`, which is not enabled | nothing is built or linked; no `links` conflict in the workspace | `Cargo.lock`; `mongocrypt-sys` `Cargo.toml` |
| Docker Official Image `mongo:8.0.34` (updated 2026-10-17); the newest tag of the 8.x line is `8.3.13` | the integration tests and the CI service run `mongo:8.0.34` | [Docker Hub tags](https://hub.docker.com/_/mongo/tags?name=8.) |
| Error codes: `HostUnreachable` 6, `HostNotFound` 7, `AuthenticationFailed` 18, `LockTimeout` 24, `CursorNotFound` 43, `MaxTimeMSExpired` 50, `NetworkTimeout` 89, `ShutdownInProgress` 91, `WriteConflict` 112, `ExceededMemoryLimit` 146, `PrimarySteppedDown` 189, `CursorKilled` 237, `ExceededTimeLimit` 262, `QueryExceededMemoryLimitNoDiskUseAllowed` 292, `SocketException` 9001, `NotWritablePrimary` 10107, `BSONObjectTooLarge` 10334, `InterruptedAtShutdown` 11600, `Interrupted` 11601, `InterruptedDueToReplStateChange` 11602, `NotPrimaryNoSecondaryOk` 13435, `NotPrimaryOrSecondary` 13436 | the driver classifies by code and label, never by message | [`error_codes.yml` at `r8.0.15`](https://github.com/mongodb/mongo/blob/r8.0.15/src/mongo/base/error_codes.yml) |
| The command reference lists every name of `oxyn-query`'s read, write, DDL and grant tables, `commitTransaction` and `abortTransaction`; it no longer lists `isMaster` (superseded by `hello`), which the tables leave out | an absent name is `Unknown`, hence mutating | [Database Commands](https://www.mongodb.com/docs/manual/reference/command/) |
| "Killing a session kills any in-progress operations in the session and closes any open cursors associated with these operations" | cancellation is `killSessions` on the execution's own logical session; a `getMore` after it gets `CursorNotFound` (43), measured on 8.0.34 | [killSessions](https://www.mongodb.com/docs/manual/reference/command/killSessions/) |
| `killSessions`: "If the user has no session that match, the killSessions has no effect" | connecting sends it for a fresh UUID nobody holds: an answer proves the session can stop its commands, at no cost | [killSessions](https://www.mongodb.com/docs/manual/reference/command/killSessions/) |
| Database names "cannot be empty and must be less than 64 bytes", and cannot contain `/\. "$` on Unix and Linux (Windows adds `*<>:\|?`) nor the null character; "MongoDB supports no more than 100 levels of nesting for BSON documents" | `$db` and the context selector refuse a name the server cannot hold; a command or reply nested deeper than 200 levels is refused before recursion | [MongoDB Limits and Thresholds](https://www.mongodb.com/docs/manual/reference/limits/) |
| `createUser` and `updateUser` carry the password in their `pwd` field, "The user's password", sent "in cleartext" — checked 2026-10-24 | a command holding a `pwd` key at any depth gets errors without the server's text: only the code and the labels Oxyn reads; the value of that key is persisted as `"<redacted>"` | [createUser](https://www.mongodb.com/docs/manual/reference/command/createUser/), [updateUser](https://www.mongodb.com/docs/manual/reference/command/updateUser/) |
| Authentication runs `{saslStart: 1, mechanism, payload: BinData(...)}` then `{saslContinue: 1, conversationId, payload: BinData(...)}`; under `PLAIN` the payload is `authzid\0user\0password`, under SCRAM the client's messages, the last carrying the proof derived from the password — checked 2026-10-24 | the `payload` of those two commands is persisted as `"<redacted>"`, whatever its Extended JSON form | [Authentication specification](https://github.com/mongodb/specifications/blob/master/source/auth/auth.md) |
| `serde_json` 1.0.151 starts its reader with `remaining_depth: 128` | a console command nests at most 128 arrays or objects; the text and value bounds of `oxyn-query` hold its size | `src/de.rs` at 1.0.151 |
| `killSessions`: "If access control is enabled, the command only kills the sessions owned by the user"; "This command is not supported in M0 and Flex clusters" | no privilege is needed to stop one's own command; a session whose server refuses the probe does not declare `SERVER_SIDE_CANCEL` | [killSessions](https://www.mongodb.com/docs/manual/reference/command/killSessions/) |
| Extended JSON v2: canonical and relaxed formats (`{"$numberInt": "42"}`, `{"$numberDecimal": "1234.5"}`…); "a parser MUST accept values in either Canonical Extended JSON format or Relaxed Extended JSON format", DBPointer included | the console reads both; it refuses DBPointer, deprecated — a documented deviation | [Extended JSON specification](https://github.com/mongodb/specifications/blob/master/source/extended-json/extended-json.md) |
| `$currentOp` lists a cursor idle between two batches only with `idleCursors: true`, as `type: "idleCursor"` with the command under `cursor.originatingCommand`; measured on 8.0.34 | the cancellation tests look there for an open cursor | measurement with `mongosh` on `mongo:8.0.34` |
| `find` and `aggregate` default to a first batch of 101 documents; "Subsequent getMore operations have no default limit, only the 16 MiB message size" | the client holds at most one server batch of 16 MiB plus one Arrow batch | [Cursors](https://www.mongodb.com/docs/manual/core/cursors/) |
| "The maximum BSON document size is 16 mebibytes"; the server's refusal reads "Size must be between 0 and 16793600(16MB)" on 8.0.34. Since 6.0, `allowDiskUseByDefault` decides whether a stage needing more than 100 megabytes spills; `{ allowDiskUse: false }` makes it fail | `BSONObjectTooLarge` and the memory-limit codes carry their remedy in the error | [MongoDB Limits and Thresholds](https://www.mongodb.com/docs/manual/reference/limits/) |
| The built-in `read` role grants `changeStream collStats dbHash dbStats find killCursors listCollections listIndexes listSearchIndexes planCacheRead`; `readAnyDatabase` adds `listDatabases` — `connectionStatus` with `showPrivileges`, measured on 8.0.34 | a session whose every privilege action is among these declares itself read-only | measurement on `mongo:8.0.34` |
| The command pages list the fields of `find`, `aggregate`, `count`, `distinct`, `insert`, `update` (and of an `updates` statement), `delete` (and of a `deletes` statement), `findAndModify`, `listCollections`, `createIndexes` (and of an index specification) and `explain`, whose `verbosity` is `queryPlanner`, `executionStats` or `allPlansExecution` (the default) — the last two run the winning plan; the operator indexes list 30 query predicates, 22 update operators and modifiers, the pipeline stages of `db.collection.aggregate()` and 25 `$group` accumulators | the console completes a chosen subset with a one-line hint (`apps/desktop/src/features/consoles/languages/mongo/vocabulary.ts`); `$where`, `$function` and `$accumulator` are not offered | [Database Commands](https://www.mongodb.com/docs/manual/reference/command/), [Query Predicates](https://www.mongodb.com/docs/manual/reference/operator/query/), [Update Operators](https://www.mongodb.com/docs/manual/reference/operator/update/), [Aggregation Stages](https://www.mongodb.com/docs/manual/reference/operator/aggregation-pipeline/), [`$group`](https://www.mongodb.com/docs/manual/reference/operator/aggregation/group/) |

## Redis protocol and client — checked on 2026-10-22

For [ADR-0061](adr/0061-redis-driver-speaks-resp3-itself.md).

| Fact | Consequence | Source |
|---|---|---|
| `redis:8.10.2` (Docker Hub, updated 2026-10-22, digest `sha256:97ec8815…`); the same day's tags also carry 8.8.3, 8.6.7, 8.4.7, 8.2.10, 7.4.11, 7.2.16 | tests and CI run `redis:8.10.2` | Docker Hub API `repositories/library/redis/tags` |
| `valkey/valkey:9.1.2` (Docker Hub, 2026-09-21, digest `sha256:418652cf…`) | the suite also runs against Valkey | Docker Hub API `repositories/valkey/valkey/tags` |
| RESP3 types: `+ - : $ * _ # , ( ! = % \| ~ >`; `$-1` and `*-1` are RESP2 nulls; doubles include `inf`, `-inf`, `nan`; a verbatim string is three format bytes and a colon; "Redis doesn't emit streamed strings" nor streamed aggregates; a bulk string is limited to 512 MB by default (`proto-max-bulk-len`); the error prefix "is a convention used by Redis rather than part of the RESP error type" | the reader refuses streamed types, and classifies errors by prefix | [protocol-spec](https://redis.io/docs/latest/develop/reference/protocol-spec/) |
| `HELLO 3` answers a map with `server`, `version`, `proto`, `id`, `mode` (`standalone`, `sentinel`, `cluster`), `role`, `modules`; an unknown `HELLO` answers `-ERR unknown command`, a refused version `-NOPROTO` | one round trip identifies the server and gives the connection id | same, § Client handshake |
| `COMMAND` returns per command: name, arity, flags, first/last key, step, ACL categories (6.0), tips, key specifications, subcommands (7.0); flags include `readonly` ("the command doesn't modify data"), `write` ("may modify data"), `blocking`, `admin`, `pubsub`, `noscript`, `no_multi` | the intent is read from `readonly`, `write` and the categories | [COMMAND](https://redis.io/docs/latest/commands/command/), [COMMAND INFO](https://redis.io/docs/latest/commands/command-info/) |
| Redis 8.10.2 answers 447 for `COMMAND COUNT`; `KEYS` is `readonly` + `@dangerous`, `FLUSHALL` `write` + `@dangerous`, `EVAL` neither `readonly` nor `write` (`@scripting`), `EVAL_RO` `readonly`, `PING` only `fast`, `INFO` `@dangerous`, `BLPOP` `write` + `blocking`, `XREAD` `readonly` + `blocking`, `SUBSCRIBE` `pubsub`, `MULTI`/`WATCH`/`AUTH`/`HELLO` `allow_busy`, every `ACL` subcommand without `readonly` | the classification table of ADR-0061 | `redis-cli COMMAND INFO …` against `redis:8.10.2`, 2026-10-22 |
| Observed on `redis:8.10.2` and `valkey/valkey:9.1.2`, 2026-10-24: `FLUSHALL`, `FLUSHDB` `write` + `@dangerous`, `SWAPDB` `write fast` + `@dangerous`; `FUNCTION FLUSH`, `DELETE`, `RESTORE` and `XGROUP DESTROY`, `DELCONSUMER` `write`, outside `@dangerous`; `ACL DELUSER` `admin` + `@dangerous`; `SCRIPT FLUSH` neither `readonly` nor `write` (`noscript`, plus `stale` on Valkey); on Redis only, `FT.DROPINDEX` and `FT.DROP` `write module` + `@dangerous`, `JSON.DEL` `write module` outside it | the risks of ADR-0066: a dangerous `write` whose reach the driver knows is a `Write` carrying its risk; `SCRIPT FLUSH` proves nothing and stays `Unknown`, its risk never consulted | `redis-cli`/`valkey-cli COMMAND INFO …` against both servers |
| Observed on `redis:8.10.2` and `valkey/valkey:9.1.2`, 2026-10-24, the whole `COMMAND` reply: 92 commands or subcommands are flagged `readonly` outside `@dangerous` on both servers; Redis alone adds 164 — `SDIFFCARD`, `SUNIONCARD`, `DIGEST`, the `AR*` array commands, and the modules' — and Valkey alone `CLUSTERSCAN`. Among Redis's, the search module flags `readonly` `FT.CURSOR DEL`, `FT.CURSOR GC` and every `_FT.DEBUG` subcommand, `GC_FORCEINVOKE`, `SET_MAX_INDEXES` and `TTL_EXPIRE` included; the probabilistic module flags `CF.COMPACT`; time series and search flag `TIMESERIES.CLUSTERSET`, `SEARCH.CLUSTERSET` and `SEARCH.CLUSTERREFRESH` | the server's `readonly` flag does not prove a read: `reads.rs` compiles in 157 proven reads — 91 of the 92, `LOLWUT` left out; `SDIFFCARD` and `SUNIONCARD`; 64 reads of the JSON, search, time series, probabilistic and vector set modules —, and a read needs the list and the flag (ADR-0061, point 3) | `redis-cli --json COMMAND`, `valkey-cli --json COMMAND` |
| `CLIENT UNBLOCK <id> ERROR` unblocks a client blocked by `BRPOP`, `XREAD`, `WAIT`…, which then reads `-UNBLOCKED`; it is `@admin @dangerous` and answers `1` or `0` | the stop of a blocked command, and its probe | [CLIENT UNBLOCK](https://redis.io/docs/latest/commands/client-unblock/) |
| `scriptKill`: `-NOTBUSY` when no script runs; `-UNKILLABLE` when the script already wrote (`SCRIPT_WRITE_DIRTY`) or came from a master; `SCRIPT KILL` refuses a function and `FUNCTION KILL` a script | a killed script wrote nothing; a script that wrote cannot be stopped | `src/script.c`, Redis 8.10.2 |
| Observed on `redis:8.10.2`, 2026-10-23: while a script runs, nothing is answered before `busy-reply-threshold` (5000 ms by default), then only `allow_busy` commands are (`MULTI`, `RESET`, `DISCARD`, `AUTH`, `UNWATCH`, `SCRIPT KILL`, `FUNCTION KILL`, `FUNCTION STATS`, `WATCH`, `SHUTDOWN`, `HELLO`, `REPLCONF`, `QUIT`); `FUNCTION STATS` shows a running `FCALL`'s `name` and `command`, but answers `-BUSY Redis is busy running a script…` while an `EVAL` runs; a killed `EVAL` and a killed `FCALL` both read `-ERR Script killed by user with SCRIPT KILL...` | an `EVAL` is identified by elimination, a function by its command line; the kill is proven by that error alone; a stop takes at least the threshold | `redis-cli` against `redis:8.10.2`; `COMMAND` filtered on `allow_busy` |
| Observed on `redis:8.10.2`, 2026-10-23: `SUBSCRIBE`, `PSUBSCRIBE` and `SSUBSCRIBE` are flagged `denyoom pubsub noscript loading stale` (`@pubsub @slow`), `MONITOR` `admin noscript loading stale` (`@admin @slow @dangerous`); `MONITOR` answers `+OK`, then one simple string per command, `1792773499.957991 [0 127.0.0.1:40342] "PING"` (`[0 lua]` for a script's call), shows `AUTH` as `"AUTH" "(redacted)" "(redacted)"` and does not show `ACL SETUSER` or `CONFIG SET`; `client-output-buffer-limit` is `pubsub 33554432 8388608 60` by default | captures keep the server's intent (`Unknown`); a `MONITOR` line splits into time, database, client and command; a subscription capture that falls behind is closed by the server; nothing here bounds `MONITOR`'s output buffer, so its window and row limit do | `redis-cli COMMAND INFO`, `MONITOR`, `CONFIG GET` against `redis:8.10.2` |
| Observed on `redis:8.10.2`, 2026-10-23: `CLIENT LIST` answers one line per client of space-separated `name=value` fields — `id addr laddr fd name age idle flags db sub psub ssub multi watch qbuf qbuf-free argv-mem multi-mem rbs rbp obl oll omem omem-shared omem-unshared tot-mem events cmd user redir resp lib-name lib-ver io-thread tot-net-in tot-net-out tot-cmds read-events avg-pipeline-len-sum avg-pipeline-len-cnt`, `age` and `idle` in seconds; a `SLOWLOG GET` entry is `[id, unix time in seconds, duration in microseconds, [arguments], client address, client name]` | the client grid takes `id addr name user db cmd flags age idle` and keeps the line; the slow log grid takes the six entry fields | `redis-cli CLIENT LIST`, `SLOWLOG GET` against `redis:8.10.2` |
| Observed on `redis:8.10.2`, 2026-10-23: under RESP3, `XINFO GROUPS` answers one map per group (`name consumers pending last-delivered-id entries-read lag`) and `XINFO CONSUMERS` one per consumer (`name pending idle inactive`); a missing key answers `-ERR no such key` | an array of maps becomes a grid whose columns are the first map's keys | `redis-cli -3 XINFO …` against `redis:8.10.2` |
| Error prefixes written by Redis 8.10.2: `ERR`, `WRONGTYPE`, `NOAUTH`, `WRONGPASS`, `NOPERM`, `NOPROTO`, `NOSCRIPT`, `NOGROUP`, `BUSYGROUP`, `BUSYKEY`, `EXECABORT`, `LOADING`, `BUSY`, `OOM`, `MISCONF`, `MASTERDOWN`, `NOREPLICAS`, `READONLY`, `TRYAGAIN`, `CLUSTERDOWN`, `CROSSSLOT`, `MOVED`, `ASK`, `UNBLOCKED`, `UNKILLABLE`, `NOTBUSY`, `DENIED`, `IOERR` | the class table of `error.rs` | `src/server.c`, `networking.c`, `cluster.c`, `script.c`, `t_stream.c`, `acl.c`, `blocked.c`, Redis 8.10.2 |
| `sdssplitargs` (`src/sds.c`, Redis 8.10.2): blanks separate; in double quotes `\xHH`, `\n`, `\r`, `\t`, `\b`, `\a` and `\<c>`; in single quotes only `\'`; a closing quote must be followed by a blank or the end; `sdscatrepr` escapes `\\`, `"`, `\n`, `\r`, `\t`, `\a`, `\b` and writes other non-printable bytes `\xHH` | the console reads a line as `redis-cli` does | source |
| `redis` (redis-rs) 1.7.1, BSD-3-Clause, `rust-version` 1.88, published 2026-09-25: `send_packed_command` returns `RedisResult<Value>`, `Value::Array(Vec<Value>)`; connection setup sends `CLIENT SETINFO LIB-NAME`/`LIB-VER` unless skipped | not a dependency: a reply is materialized before the caller sees it | crates.io API; `src/aio/multiplexed_connection.rs`, `src/types.rs`, `src/connection.rs` at 1.7.1 |
| `redis` 1.8.0, published 2026-10-23, still returns a `Value` whose `Array`, `Map`, `Set` and `Push` variants hold every element | the rejection of ADR-0061 holds on the latest release | docs.rs `redis/1.8.0/redis/enum.Value.html`, checked 2026-10-23 |
| `TYPE` returns `string`, `list`, `set`, `zset`, `hash`, `stream` and `vectorset`, or `none`; vector sets come with Redis 8.0 (`VADD key (FP32 \| VALUES num) vector element`, `VCARD`); `VRANGE key start end [count]` (since 8.4.0, `READONLY`) returns elements in byte order, `-`/`+` the bounds, `[`/`(` an inclusive/exclusive start | a vector set previews with `VRANGE key - + limit` when the server knows the command, its length is `VCARD` |
| `redis:8.10.2` loads the modules `search`, `vectorset`, `bf`, `timeseries` and `ReJSON` (`MODULE LIST`); `TYPE` names their keys `ReJSON-RL`, `TSDB-TYPE`, `MBbloom--`, `MBbloomCF`, `CMSk-TYPE`, `TopK-TYPE`, `TDIS-TYPE`; `JSON.GET`, `TS.RANGE`, `BF.INFO`, `CF.INFO`, `CMS.INFO`, `TOPK.LIST`, `TDIGEST.INFO` are flagged `readonly` and `module`, none `@dangerous`; in RESP3 `TS.RANGE` answers `[[:timestamp, ,value]…]` and `BF.INFO` a map | a module type previews with its module's read, `Read` because the driver's list holds it and the server flags it `readonly` | `redis-cli -3 MODULE LIST`, `TYPE`, `COMMAND INFO` against `redis:8.10.2`, 2026-10-23 | [TYPE](https://redis.io/docs/latest/commands/type/), [VADD](https://redis.io/docs/latest/commands/vadd/), [VRANGE](https://redis.io/docs/latest/commands/vrange/), checked 2026-10-23 |
| `tokio-rustls` 0.26.6 is the latest release (2026-09-28); the lockfile already resolves 0.26.5 (MIT OR Apache-2.0, `rust-version` 1.71) for `sqlx`; features `aws_lc_rs`, `tls12` | the driver takes the release in the tree, on the workspace's provider | crates.io API |
| `webpki-roots` 1.0.10, CDLA-Permissive-2.0, already accepted by `deny.toml` | the public roots of `verify-full` | crates.io API |

| Crate | Version | Latest | Source | Checked on |
|---|---|---|---|---|
| `tokio-rustls` | `0.26.5` | `0.26.6` (2026-09-28) | [crates.io](https://crates.io/crates/tokio-rustls) | 2026-10-22 |
| `webpki-roots` | `1.0.10` | `1.0.11` (2026-10-23) | [crates.io](https://crates.io/crates/webpki-roots) | 2026-10-23 |


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

## PostgreSQL EXPLAIN options and classification — checked 2026-10-22

PostgreSQL **18** (documentation 18.6,
[`EXPLAIN`](https://www.postgresql.org/docs/18/sql-explain.html)) lists twelve
parenthesized options: `ANALYZE`, `VERBOSE`, `COSTS`, `SETTINGS`,
`GENERIC_PLAN`, `BUFFERS`, `SERIALIZE`, `WAL`, `TIMING`, `SUMMARY`, `MEMORY`,
`FORMAT`. Only `ANALYZE` runs the statement; `SERIALIZE`, `WAL` and `TIMING`
are refused without it, and `GENERIC_PLAN` is refused with it. The grammar
(`src/backend/parser/gram.y`, branch `REL_18_STABLE`, rules `analyze_keyword`
and `utility_option_name`) accepts `ANALYSE` as well as `ANALYZE`, in the
option list and in the legacy `EXPLAIN ANALYSE statement` form, and gives
both the option name `analyze`; any other non-reserved word is passed on as
an option name.

`sqlparser` **0.62.0** has no `ANALYSE` keyword (`src/keywords.rs`): in the
parenthesized form it is a `UtilityOption` named `ANALYSE`, and the legacy
form does not parse. The classification therefore keeps `Read` only for the
eleven options that shape the plan, and gives any other option the explained
statement's intent (issue #246).

The [`COPY`](https://www.postgresql.org/docs/18/sql-copy.html) page, read the
same day: its `query` may be a `SELECT`, `VALUES`, `INSERT`, `UPDATE`,
`DELETE` or `MERGE`, the last four with a `RETURNING` clause — a `COPY (…) TO
STDOUT` can therefore write. No live database was used for this verification.

## SQL password literals before persistence

Checked on 2026-10-03 for [ADR-0053](adr/0053-redact-sql-passwords-before-persistence.md).

- PostgreSQL [ALTER ROLE](https://www.postgresql.org/docs/current/sql-alterrole.html)
  admits password string literals and `PASSWORD NULL`; its
  [lexical rules](https://www.postgresql.org/docs/current/sql-syntax-lexical.html)
  describe escape, Unicode and dollar-quoted strings, adjacent string fragments,
  and the effect of `standard_conforming_strings`.
- MySQL [CREATE USER](https://dev.mysql.com/doc/refman/8.4/en/create-user.html),
  [ALTER USER](https://dev.mysql.com/doc/refman/8.4/en/alter-user.html), and
  [SET PASSWORD](https://dev.mysql.com/doc/refman/8.4/en/set-password.html)
  describe `IDENTIFIED BY`, plugin authentication clauses, quoted accounts,
  password assignments and the old password supplied by `REPLACE`.
  [String literals](https://dev.mysql.com/doc/refman/8.4/en/string-literals.html)
  describes double quotes, character-set introducers, adjacent strings and
  `NO_BACKSLASH_ESCAPES`.
- MariaDB [CREATE USER](https://mariadb.com/docs/server/reference/sql-statements/account-management-sql-statements/create-user)
  also permits `OR REPLACE` and `IDENTIFIED VIA … USING PASSWORD(…)`;
  [GRANT](https://mariadb.com/docs/server/reference/sql-statements/account-management-sql-statements/grant)
  can carry an authentication clause. These use the same MySQL driver/dialect.
- MySQL [comments](https://dev.mysql.com/doc/refman/8.4/en/comments.html)
  distinguishes executable `/*! … */` content from ordinary comments; a password
  clause there is still SQL and must be scanned before persistence.

The redactor reuses the workspace scanner and combines lexical interpretations
where SQL mode or dialect is unavailable. This is a persistence/context copy;
no redacted text is submitted to a database. Grammar and executor-boundary tests
use synthetic credentials and a recording session; they do not certify a live
server execution.

MariaDB also documents its `/*M! … */` form in [Comment Syntax](https://mariadb.com/docs/server/reference/sql-statements/comment-syntax), checked on the same date; both executable comment forms are scanned.

## PostgreSQL TLS modes — checked on 2026-10-03

For [ADR-0052](adr/0052-verified-tls-outside-local.md), checked the official
[SSL support documentation](https://www.postgresql.org/docs/18/libpq-ssl.html),
sections on certificate verification and protection by mode: `verify-full`
checks the certificate chain and hostname; `verify-ca` checks the chain alone.
`prefer` does not protect against an active intermediary. Oxyn's policy applies
these distinctions to the connection environment; no dependency version changes.

The pinned registry sources and workspace features also confirm the trust store:
`sqlx-core`'s `_tls-rustls-aws-lc-rs` enables `webpki-roots`, and
`src/net/tls/tls_rustls.rs::import_root_certs` imports that bundled set;
`mysql_async`'s `src/io/tls/rustls_io.rs` likewise imports
`webpki_roots::TLS_SERVER_ROOTS`. Neither shipped driver selects a system-root
feature or exposes a per-connection CA field. PostgreSQL's `sslrootcert`
configuration key currently falls into `ConnectSpec`'s server options.

## External-agent declaration characters — checked 2026-10-03

The Unicode Character Database's [UnicodeData.txt](https://www.unicode.org/Public/UCD/latest/ucd/UnicodeData.txt)
was checked for general categories `Cf`, `Zl` and `Zp`. They include U+2028,
U+2029, bidi controls and zero-width format characters that `char::is_control`
does not reject. `ExternalAgentConfig::validate` rejects these ranges in every
text field, alongside the existing invisible-character ranges used by native
confirmations. The 4,096-byte command/argument bounds are Oxyn product limits,
not operating-system limits; no dependency was added.

## Idle streams — checked on 2026-10-04

Question: does a provider promise to send something at a known interval while
the model thinks silently, so that a bound on silence could follow it? The
answer sets the idle bounds of
[AI-PROVIDERS](AI-PROVIDERS.md#a-silent-provider).

| Fact | Source | Checked on |
|---|---|---|
| Anthropic sends `ping` events "in any number"; no interval is given | [Streaming](https://platform.claude.com/docs/en/build-with-claude/streaming), recorded above in "SSE stream" | 2026-09-16 |
| Anthropic: "Some networks may drop idle connections after a variable period of time"; streaming or batches are advised for long requests, "especially those over 10 minutes", and a TCP keep-alive for direct integrations. No interval for `ping` there either | [Claude API errors § Long requests](https://platform.claude.com/docs/en/api/errors#long-requests) | 2026-10-04 |
| llama.cpp sends `:` SSE keep-alive comments; no interval was recorded | recorded above in "End of stream on local OpenAI-compatible servers" | 2026-09-16 |
| OpenAI: nothing about keep-alives is recorded in this file, and it was not re-checked | — | — |
| reqwest **0.13.4** `ClientBuilder::read_timeout`: "applies to each read operation, and resets after a successful read". The same timer also bounds the wait for the response headers (`Pending::poll`); past it, a body read fails with `is_timeout()` | installed sources, `src/async_impl/client.rs`; behavior checked by `crates/oxyn-llm/src/http/idle_tests.rs` | 2026-10-04 |

Consequence retained: there is no published interval to follow. The 300 s
bound is Oxyn's, long enough for a model that thinks for minutes, and it is
reset by any byte — a `ping` or a `:` comment counts. The same holds for
external agents: nothing in this file records how often an adapter reports
progress, and their bound is a product choice too.

## sqlite-vec — checked on 2026-10-06

Question: can the SQLite driver read a `CREATE VIRTUAL TABLE … USING vec0(…)`
table without loading a native file at runtime? The answer grounds
[ADR-0054](adr/0054-bundle-sqlite-vec-in-the-sqlite-driver.md).

| Fact | Source | Checked on |
|---|---|---|
| `sqlite-vec` crate: latest stable **0.1.9**, published 2026-03-31; the 0.1.10 line is pre-release only (`0.1.10-alpha.4`, 2026-05-18) | [crates.io API](https://crates.io/api/v1/crates/sqlite-vec), [GitHub releases](https://github.com/asg017/sqlite-vec/releases) | 2026-10-06 |
| License declared `MIT/Apache-2.0`; the repository ships `LICENSE-MIT` and `LICENSE-APACHE`. `cargo deny check licenses bans` passes with it | crate manifest; [repository](https://github.com/asg017/sqlite-vec); local run | 2026-10-06 |
| Maintenance: not archived, last push 2026-05-18, 211 open issues, no published security advisory, no RustSec entry | [GitHub API](https://api.github.com/repos/asg017/sqlite-vec), [rustsec.org](https://rustsec.org/) | 2026-10-06 |
| The crate has no dependency; its `build.rs` compiles `sqlite-vec.c` (320 026 bytes) with `cc` and the only define `SQLITE_CORE` — no SIMD flag, so the scalar code paths. `cc` 1.4.5 is already in `Cargo.lock` | crate source `build.rs` | 2026-10-06 |
| With `SQLITE_CORE`, the extension calls `sqlite3_*` directly: they resolve, at link time, to the SQLite 3.50.2 that `libsqlite3-sys` 0.35.0 `bundled` compiles. The crate compiles against its own `sqlite3.h` 3.45.3, an older header: SQLite keeps its C interface backward compatible | crate source `sqlite-vec.c`, `sqlite3.h`; `libsqlite3-sys-0.35.0/sqlite3/sqlite3.h`; [SQLite compatibility](https://www.sqlite.org/capi3ref.html) | 2026-10-06 |
| `src/lib.rs` declares `pub fn sqlite3_vec_init();` with no argument; the C definition is `int sqlite3_vec_init(sqlite3 *db, char **pzErrMsg, const sqlite3_api_routines *pApi)`, and on failure it writes `*pzErrMsg` without checking for null | crate source `src/lib.rs`, `sqlite-vec.c` l. 10064–10145 | 2026-10-06 |
| `sqlite3_vec_init` registers functions (`vec_f32`, `vec_distance_*`, `vec_to_json`…) and the modules `vec0` and `vec_each`. `vec_npy_file`, which opens a file by path, belongs to `sqlite3_vec_numpy_init`, a separate entry point | crate source `sqlite-vec.c` l. 10064–10165 | 2026-10-06 |
| `rusqlite` 0.37.0 offers no safe registration of a statically linked extension: `Connection::handle` and `auto_extension::register_auto_extension` are `unsafe`; `load_extension` loads a file | installed source `rusqlite-0.37.0/src/lib.rs`, `src/auto_extension.rs` | 2026-10-06 |
| A `float[N]` vector is stored and returned as a BLOB of N little-endian `f32` | observed by `drivers/oxyn-driver-sqlite/src/vector_tests.rs` | 2026-10-06 |
| The release CI builds macOS on `macos-latest` (arm64) and Linux on `ubuntu-24.04` and `ubuntu-24.04-arm`; pull requests build Linux x86_64 only. The crate was built and tested locally on macOS arm64 | `.github/workflows/livraison.yml`, `qualite.yml`; local run | 2026-10-06 |
| Release build of the crate alone: 8.2 s wall time on an Apple arm64 laptop; the static archive holds 96 246 bytes of `__TEXT` and 1 152 of `__DATA` | `cargo build -p sqlite-vec --release`, `size` on `libsqlite_vec0.a` | 2026-10-06 |
| Stripped release `oxyn-desktop` (macOS arm64, `lto = "thin"`): 26 535 280 bytes with the extension, 26 450 800 without, i.e. +84 480 bytes; the relink after removing it took 3 min 30 s | two `cargo build --release -p oxyn-desktop` of the same tree | 2026-10-06 |
| `assert()` stays live: `build.rs` does not define `NDEBUG`, the release archive imports `___assert_rtn`; `sqlite-vec.c` holds 42 `assert(` calls, some on sizes read from shadow tables (`vec0_metadata_filter_text`, l. 6155–6156) | `nm -u` on `libsqlite_vec0.a`; crate source | 2026-10-06 |
| An interrupted `INSERT` into `vec0` can fail with `SQLITE_ERROR`, "Internal sqlite-vec error: Could not find latest chunk", instead of `SQLITE_INTERRUPT` | observed by `a_stopped_write_into_vec0_is_ambiguous` before the fix (one run in three) | 2026-10-06 |
| sqlite-vec never calls `sqlite3_vtab_config`: `vec0` is marked neither `SQLITE_VTAB_INNOCUOUS` nor `SQLITE_VTAB_DIRECTONLY`. `libsqlite3-sys` 0.35.0's `build.rs` does not set `SQLITE_TRUSTED_SCHEMA`, so the engine default (on) applies | crate source; `libsqlite3-sys-0.35.0/build.rs` | 2026-10-06 |

## Local embeddings — checked on 2026-10-07

Question: can Oxyn compute text embeddings in its own process, on the CPU,
with nothing installed beside it, well enough to rank catalog relations
against a question? The answer grounds
[ADR-0056](adr/0056-local-cpu-embeddings-for-context-selection.md). The
measurements come from a spike run on 2026-10-07 on an Apple Silicon arm64
laptop with 10 cores, in release; they were not repeated on Linux.

### Crates

| Fact | Source | Checked on |
|---|---|---|
| `burn`, `burn-flex`, `burn-store`, `burn-onnx`: latest stable **0.22.0**, all four published 2026-10-06, `MIT OR Apache-2.0`; `burn` declares `rust-version = "1.95"`, the workspace floor | [crates.io API](https://crates.io/api/v1/crates/burn) | 2026-10-07 |
| `burn` with `default-features = false` and `flex` enables burn-flex with `std` only; `simd` and `rayon` need burn-flex declared directly | `oxyn-embed` spike; root `Cargo.toml` comment | 2026-10-07 |
| `tokenizers`: latest stable **0.23.2**, published 2026-09-03, `Apache-2.0`; a `1.0.0-rc.2` pre-release exists (2026-09-21) | [crates.io API](https://crates.io/api/v1/crates/tokenizers) | 2026-10-07 |
| `tokenizers` with `default-features = false` and `fancy-regex` gives the same token ids as the default Oniguruma build on 17 multilingual cases | spike | 2026-10-07 |
| Resolved graph of the inference crate: 198 crates, no `-sys` crate; the only MPL-2.0 license, `colored`, is accepted by `deny.toml` | `cargo tree`, `cargo deny check licenses` in the spike | 2026-10-07 |
| `ort`: no stable 2.0, latest **2.0.0-rc.13** (2026-07-28); depends on `ort-sys =2.0.0-rc.13`; default features include `download-binaries` (prebuilt onnxruntime fetched at build time), `load-dynamic` is the alternative | [crates.io API](https://crates.io/api/v1/crates/ort/2.0.0-rc.13) | 2026-10-07 |
| `candle-transformers`: latest stable **0.11.0** (2026-06-26); `src/models/modernbert.rs` implements ModernBERT with `global_rope_theta`, `local_rope_theta` and the local attention window from the configuration. Not measured | crate source `candle-transformers-0.11.0` | 2026-10-07 |

### The model

| Fact | Source | Checked on |
|---|---|---|
| `ibm-granite/granite-embedding-97m-multilingual-r2`: license `apache-2.0`, pipeline `feature-extraction`, created 2026-04-20, last modified 2026-05-18; pinned revision `835ad14087e140460703cf0fae09f97d469d65c2` | [Hugging Face API](https://huggingface.co/api/models/ibm-granite/granite-embedding-97m-multilingual-r2/revision/835ad14087e140460703cf0fae09f97d469d65c2) | 2026-10-07 |
| At that revision: `model.safetensors` 194 889 568 bytes, SHA-256 `f3ea88b230492811046145513710e76b4cc8c2ad49e8708da0e7247e548903be`; `tokenizer.json` 25 301 672 bytes, SHA-256 `4f2842d568e2724370aec203652a42ac783c7937f8347a1a2cc7506d71f1582f`; `onnx/model.onnx` 390 004 608 bytes, SHA-256 `68e592b160673d30250824c1116bc6ab33f70efb22b97c9e1d7ce1e69c1c9d70` | same API, `?blobs=true` (LFS checksums) | 2026-10-07 |
| The safetensors holds 74 tensors, all `BF16`, 97 441 152 parameters: 389 764 608 bytes once widened to f32 | header of `model.safetensors`, read by range request | 2026-10-07 |
| `config.json`: `ModernBertModel`, `hidden_size` 384, 12 layers, 12 heads, `vocab_size` 180 000, `global_rope_theta` 150 000, `local_rope_theta` 160 000, `local_attention` 128, global attention every 3 layers; `1_Pooling/config.json`: CLS pooling, 384 dimensions | files at the pinned revision | 2026-10-07 |
| The repository has no `LICENSE` file: the license is declared in the model card metadata | file list at the pinned revision | 2026-10-07 |
| `resolve/<revision>/model.safetensors` answers `302` towards `us.aws.cdn.hf.co`, with `x-linked-size` and `x-linked-etag` equal to the size and SHA-256 above | `curl -I` | 2026-10-07 |

### Measured by the spike

| Fact | Source | Checked on |
|---|---|---|
| burn-onnx 0.22.0 converts the ONNX export without a missing operator; 43 parameters are constant-folded ONNX initializers absent from the safetensors (rotary frequencies, LayerNorm biases, scalars) | spike | 2026-10-07 |
| Burn output against onnxruntime on the same inputs: maximum absolute gap 4.2e-7, cosine 1.0 | spike | 2026-10-07 |
| Latency: about 14 ms for one sentence, about 440 ms for 256 table names | spike, release | 2026-10-07 |
| Peak RSS: 1.15 GB loading the safetensors; about 750 MB loading a `.bpk` in f32; the tokenizer loaded alone, 286 MB | spike, release | 2026-10-07 |
| Cosine question ↔ relevant table: 0.909 (English), 0.865 (French); irrelevant tables at most 0.773 | spike | 2026-10-07 |
| Cold compilation of the inference crate: 80 s of wall time, 45 s of which is burn-flex on the critical path; demo binary 11.7 MB, 8.6 MB stripped | spike, `cargo build --release` | 2026-10-07 |
| The generated `model.rs` holds 69 `.unwrap()`, no `expect` nor `panic!`, and some 250 `as` conversions | `grep` on `crates/oxyn-embed/src/generated/model.rs` | 2026-10-07 |

### Measured by `oxyn-embed`

Same machine, release build unless stated, commit `a7233c3`.

| Fact | Source | Checked on |
|---|---|---|
| Cold load 0.67 s (hashing the 415 MB of files, mapping, warm-up); one question 18 ms; 256 table names 0.63 s, and 2.2 s in the dev profile (about 3.5 times slower) | `oxyn-embed` ignored tests, commit `a7233c3` | 2026-10-07 |
| RSS once loaded: 795 MB — with `BurnpackStore::from_file`, which copies the file into the heap (see 2026-10-08 below) | same | 2026-10-07 |
| Real download and conversion: 14 s, RSS peak 1.45 GB; what stays on disk is `tokenizer.json` and `model.bpk`, about 415 MB, the safetensors being deleted once the conversion is verified | same | 2026-10-07 |
| `BurnpackStore` records each parameter's `ParamId`, a random `u64` drawn when the model is built: two conversions of the same weights differ by 590 header bytes. Written through `burn_pack::Writer` with every `param_id` set to `None`, `model.bpk` is reproducible: 389 816 832 bytes, SHA-256 `d5ac67b8e7e85e63ba433faebbe3e5537732ab7e27dde9cf3422710c6abce719` | `crates/oxyn-embed/src/load.rs`, `src/pinned.rs` | 2026-10-07 |

### Memory measured on 2026-10-08

A dedicated harness reading the process's footprint, RSS and `malloc`
statistics, release build, macOS 26.2, Apple M1 Max, two runs each.

| Fact | Source | Checked on |
|---|---|---|
| `BurnpackStore::from_file` reads the whole `model.bpk` into the heap through burn-pack's `Bytes::from_file`: 434 MB of `malloc`, although burn-store's `memmap` feature is on | harness; `burn-store-0.22.0/src/burnpack.rs` → `burn-pack-0.22.0/src/reader.rs` (`Source::File(Bytes::from_file(…))`) | 2026-10-08 |
| Loaded with `BurnpackStore::from_file`: footprint 553 MB, then 643 MB after embedding 256 names; RSS 677 then 766 MB | harness, commit `ea98128` message | 2026-10-08 |
| Loaded from a `memmap2` mapping of the verified file (`ea98128`): footprint 157–179 MB, then 248–268 MB after 256 names; RSS 388–409 then 484–503 MB. The vectors still match onnxruntime | same | 2026-10-08 |
| After the conversion, the process keeps a footprint of 1,187 MB with 0.2 MB of `malloc` in use: `vmmap` lists 1.1 GB of `MALLOC_LARGE (empty)`. It does not move in 25 s, nor on `malloc_zone_pressure_relief` | harness log, `vmmap` | 2026-10-08 |
| After unloading the model, about 240 MB stay held by the system allocator | harness | 2026-10-08 |
| `MallocLargeCache=0` releases those regions but makes inference 31 % slower: 2.51 s on the measured workload | harness | 2026-10-08 |
| With `mimalloc` as the global allocator, about 70 MB stay held after unloading | harness | 2026-10-08 |
| `memmap2` **0.9.11**, the version burn-store already pulls, becomes a workspace dependency; `bytes` 1.12.1 was already one | `Cargo.toml`, `Cargo.lock` at `ea98128` | 2026-10-08 |

### Dependencies brought by `oxyn-embed`

| Fact | Source | Checked on |
|---|---|---|
| `sha2` **0.11.0**, already a workspace dependency; `Cargo.lock` also keeps 0.10.9 for other crates | `Cargo.lock` at `a7233c3` | 2026-10-07 |
| The new requirements move five existing `Cargo.lock` entries within their semver range: `uuid` 1.26.0 → 1.27.0, `cc` 1.4.5 → 1.6.0, `syn` 3.0.5 → 3.0.6, `rand` 0.10.2 → 0.10.3, `find-msvc-tools` 0.1.12 → 0.1.14 | `git diff` of `Cargo.lock`, `8ec843c..a7233c3` | 2026-10-07 |
| Python packages pinned by `codegen/reference.py` and `codegen/regenerate.py` (outside the build, run with `uv`): `onnxruntime` **1.30.0** (2026-09-10), `numpy` **2.5.3** (2026-09-06), `tokenizers` **0.23.2** (2026-09-03) | [PyPI JSON API](https://pypi.org/pypi/onnxruntime/1.30.0/json) | 2026-10-07 |
| `reqwest` 0.13.4 gains the `system-proxy` feature (commit `83e8076`); through `hyper-util`, `Cargo.lock` gains four entries: `system-configuration` **0.7.0** (2025-12-02), `system-configuration-sys` **0.6.0** (2024-01-31), a second `core-foundation` **0.9.4** (2023-11-30) beside the existing 0.10.0, and `windows-registry` **0.6.1** (2025-10-06), all `MIT OR Apache-2.0`. Newer releases exist (`system-configuration` 0.8.0, `windows-registry` 0.100.0, `core-foundation` 0.10.1): these are the versions `hyper-util` asks for | `Cargo.lock` at `83e8076`; [crates.io API](https://crates.io/api/v1/crates/system-configuration) | 2026-10-07 |
| `system-configuration-sys` 0.6.0 is a `-sys` crate, unlike the spike's graph: its `build.rs` only links the system `SystemConfiguration` framework on Apple targets, and compiles no C | crate source `build.rs` | 2026-10-07 |
| `reqwest` is a single version in the graph (0.13.4), shared by `oxyn-llm`, `oxyn-embed` and `tauri-plugin-updater` 2.13.1: a feature enabled for one is enabled for all; `oxyn-llm`'s client calls `no_proxy()` (`crates/oxyn-llm/src/http.rs`) | `Cargo.lock`; source | 2026-10-07 |

### Alternatives

| Fact | Source | Checked on |
|---|---|---|
| `microsoft/harrier-oss-v1-270m`: MIT, 268 098 176 parameters in BF16, no `.onnx` file in the repository | [Hugging Face API](https://huggingface.co/api/models/microsoft/harrier-oss-v1-270m) | 2026-10-07 |
| `voyageai/voyage-4-nano`: Apache-2.0, 346 451 968 parameters in BF16; ONNX exports exist only in third-party repositories | [Hugging Face API](https://huggingface.co/api/models/voyageai/voyage-4-nano) | 2026-10-07 |
| `jinaai/jina-embeddings-v5-text-*`: CC-BY-NC-4.0 | Hugging Face API | 2026-10-07 |
| `google/embeddinggemma-300m`: Gemma license, 302 863 104 parameters | Hugging Face API | 2026-10-07 |
| `MongoDB/mdbr-leaf-mt`: Apache-2.0, 22 565 376 parameters in F32, ONNX published, language tag `en` only | Hugging Face API | 2026-10-07 |

### GitHub releases

| Fact | Source | Checked on |
|---|---|---|
| "The latest release is the most recent non-prerelease, non-draft release, sorted by the `created_at` attribute"; drafts and pre-releases cannot be set as latest | [GitHub REST API, releases](https://docs.github.com/en/rest/releases/releases#get-the-latest-release) | 2026-10-07 |
| The fallback [`embedding-model-835ad140`](https://github.com/so-keyldzn/oxyn/releases/tag/embedding-model-835ad140) is published (2026-10-07T16:45:34Z), `prerelease: true`, `draft: false`. Assets and their GitHub digests: `model.safetensors` 194 889 568 bytes `sha256:f3ea88b2…903be`, `tokenizer.json` 25 301 672 bytes `sha256:4f2842d5…1582f` — both equal to the pinned values —, `LICENSE-Apache-2.0.txt` 11 358 bytes, `NOTICE.md` 891 bytes. The latest release stayed `v0.0.7`, and `releases/latest/download/latest.json` answers 200 | `gh api repos/so-keyldzn/oxyn/releases/tags/embedding-model-835ad140`, `…/releases/latest`; `curl -L` | 2026-10-07 |

## TanStack Start family and GHSA-qx66-fv34-fjm8 — checked on 2026-10-20

This section supersedes the `@tanstack/react-start`, `react-router`, `router-plugin`
and `@tanstack/react-store` versions cited in the interface table above (`1.168.54`,
`1.170.36`, `1.168.38`, `0.11.1`): the `package.json` of `apps/desktop` now pins the
versions below. Every version and date comes from the npm registry
(`npm view @tanstack/<package> time --json`) read on 2026-10-20.

### The advisory

| Fact | Source | Checked on |
|---|---|---|
| [GHSA-qx66-fv34-fjm8](https://github.com/advisories/GHSA-qx66-fv34-fjm8) (CVE-2026-102989), severity **critical**, published 2026-10-15T20:22:20Z: an unauthenticated attacker crafts a URL to a TanStack Start server function and, if someone opens it, the application returns attacker-controlled HTML from its own origin (reflected XSS). Cause: the server-function transport passes request payload fields into internal middleware state. Affected: `@tanstack/start-server-core` `>= 1.143.12, < 1.169.39` and `@tanstack/react-start` `>= 1.143.12, < 1.168.60`, with the same range on the Solid and Vue packages. The fix limits client-supplied input to public fields and validates responses at the server boundary | `gh api /advisories/GHSA-qx66-fv34-fjm8` | 2026-10-20 |
| `pnpm audit --prod` (the `audit-npm` step of `make front-controles`) on the lockfile of `e9cd78f` reports 2 critical advisories: `@tanstack/react-start` 1.168.54 and `@tanstack/start-server-core` 1.169.35. After the change below it reports `No known vulnerabilities found` | `pnpm audit --prod` in `apps/desktop`, before and after | 2026-10-20 |
| `apps/desktop/src` defines no server function (no `createServerFn`) and Start runs in SPA mode with a prerendered `index.html` (`vite.config.ts`): the bump is required by the audit gate and removes the vulnerable code from the dependency graph, but the shipped application does not expose a server-function endpoint | `grep -rn createServerFn apps/desktop/src` at `e9cd78f` | 2026-10-20 |

### Versions pinned in `apps/desktop/package.json`

`@tanstack/react-start` pins every other package of the family **exactly**
(`npm view @tanstack/react-start@1.168.60 dependencies`): a direct pin of
`@tanstack/react-router` or `@tanstack/router-plugin` that differs from what
`react-start` pins leaves two copies of `router-core` in the lockfile.

| Package | Version | Why this version | Published | Source |
|---|---|---|---|---|
| `@tanstack/react-start` | `1.168.60` | first patched version of the 1.168 line (the smallest change that clears the advisory); replaces `1.168.54` | 2026-09-30T17:48:34Z | [npm registry](https://registry.npmjs.org/@tanstack%2Freact-start) |
| `@tanstack/react-router` | `1.170.41` | the exact version `react-start` 1.168.60 depends on; replaces `1.170.36` | 2026-09-30T17:48:34Z | [npm registry](https://registry.npmjs.org/@tanstack%2Freact-router) |
| `@tanstack/router-plugin` | `1.168.42` | the exact version `start-plugin-core` 1.171.49 depends on; its peer `@tanstack/react-router ^1.170.41` accepts the line above; replaces `1.168.38` | 2026-09-30T17:48:34Z | [npm registry](https://registry.npmjs.org/@tanstack%2Frouter-plugin) |
| `@tanstack/react-store` | `0.11.2` | `react-router` 1.170.41 requires `^0.11.2`; with `0.11.1` pinned the lockfile held two copies of `@tanstack/store`. `react-form` 1.33.5 (`^0.11.0`) and `react-pacer` 0.23.0 (`^0.11.1`) accept `0.11.2`; replaces `0.11.1` | 2026-09-29T07:59:49Z | [npm registry](https://registry.npmjs.org/@tanstack%2Freact-store) |

`@tanstack/react-router-devtools` stays at `1.167.2`: its peers
(`@tanstack/router-core ^1.171.30`, `@tanstack/react-router ^1.170.36`) accept the
new versions. `@tanstack/devtools-vite` stays at `0.8.5`, which peers on `vite` only.

### Resolved in `pnpm-lock.yaml` (not pinned in the manifest)

Moved by `pnpm install` because `react-start` 1.168.60 pins them exactly; none needs
an override.

| Package | Resolved version | Published |
|---|---|---|
| `@tanstack/start-server-core` | `1.169.39` (first patched version; was `1.169.35`) | 2026-09-30T17:48:11Z |
| `@tanstack/router-core` | `1.171.34` (was `1.171.30`) | 2026-09-30T17:50:05Z |
| `@tanstack/start-plugin-core` | `1.171.49` (was `1.171.44`) | 2026-09-30T17:49:13Z |
| `@tanstack/start-client-core` | `1.170.34` (was `1.170.30`) | 2026-09-30T17:47:20Z |
| `@tanstack/react-start-client` | `1.168.39` (was `1.168.34`) | 2026-09-30T17:51:06Z |
| `@tanstack/react-start-server` | `1.167.46` (was `1.167.41`) | 2026-09-30T17:49:03Z |
| `@tanstack/react-start-rsc` | `0.1.59` (was `0.1.53`) | 2026-09-30T17:47:30Z |
| `@tanstack/router-generator` | `1.167.40` (was `1.167.36`) | 2026-09-30T17:48:02Z |
| `@tanstack/store` | `0.11.2` (second copy removed; was `0.11.1`) | 2026-09-29T08:00:31Z |

### Why bumping `@tanstack/react-start` alone fails

| Fact | Source | Checked on |
|---|---|---|
| With only `react-start` moved to 1.168.60 (Dependabot PR #235), the direct pin `react-router` 1.170.36 (`router-core` 1.171.30) stays beside the 1.170.41 (`router-core` 1.171.34) that `react-start` brings: the lockfile holds two copies of each. `pnpm audit --prod` is clean in that state, but the production build fails in the SPA prerender step with `Error in renderToReadableStream: TypeError: router.serverSsr.takeBufferedScripts is not a function` then `Failed to fetch /: Internal Server Error`, the failure of the `rust (ubuntu-24.04)` job of #235. Reproduced in a scratch copy of `apps/desktop` outside the repository | `pnpm build` in the scratch copy; job log `gh api repos/so-keyldzn/oxyn/actions/jobs/118199466822/logs` | 2026-10-20 |
| With the four pins above, `pnpm build` (client, SSR and prerender of `/`) succeeds | `pnpm build` in `apps/desktop` | 2026-10-20 |

### Not taken

| Fact | Source | Checked on |
|---|---|---|
| react-start `1.168.61` (2026-10-10T07:29:33Z) is the latest patch of the 1.168 line, and `1.169.3` (2026-10-16T21:10:41Z) is the `latest` dist-tag; its peer is `vite >=8.0.0`. Neither is needed to clear the advisory: a later bump goes through the usual update flow | `npm view @tanstack/react-start time dist-tags --json` | 2026-10-20 |

## RustSec advisories on the locked TLS and Wasmtime crates — checked on 2026-10-20

`cargo deny --all-features check` (cargo-deny 0.19.0, advisory database read on
2026-10-20) failed on the `Cargo.lock` of `e9cd78f` with **20 advisories**,
published between 2026-10-14 and 2026-10-20, after the last green run of `main`
(2026-10-12): four on `rustls`, one on `rustls-webpki`, one on `webpki-roots`,
one on `webpki-root-certs` and thirteen on `wasmtime`. Each is cleared by a
semver-compatible update of a crate that `Cargo.lock` already held. No manifest
changed — `wasmtime = "48.0.5"` stays the declared floor and `Cargo.lock`
resolves 48.0.7 — and no advisory is ignored in `deny.toml`. After the update
the same command prints `advisories ok, bans ok, licenses ok, sources ok`. The
two 2026-10-02 sections above record what was true on their date; the versions
below supersede their `Cargo.lock` facts (`rustls` 0.23.45, Cranelift 0.135.5).

### Versions resolved in `Cargo.lock`

| Crate | Before (`e9cd78f`) | After | Published on crates.io |
|---|---|---|---|
| `rustls` | 0.23.45 | **0.23.46** | 2026-10-14 |
| `rustls-webpki` | 0.103.15 | **0.103.16** | 2026-10-20 |
| `webpki-roots` 1.x (the 0.26.11 entry is untouched and depends on 1.x) | 1.0.9 | **1.0.10** | 2026-10-19 |
| `webpki-root-certs` | 1.0.9 | **1.0.10** | 2026-10-19 |
| `wasmtime`, `wasmtime-environ`, `pulley-interpreter`, `pulley-macros` and the eleven `wasmtime-internal-*` crates, which move together | 48.0.5 | **48.0.7** | 2026-10-19 |
| the thirteen `cranelift-*` crates, which move together | 0.135.5 | **0.135.7** | 2026-10-19 |

32 `Cargo.lock` entries change version and nothing else does: no entry is added
or removed, and the only other lines that differ are the four references to
`webpki-roots` that follow the bump. None of the 32 versions is yanked. Source:
`Cargo.lock` at `e9cd78f` and in the working tree, crates.io API
`/api/v1/crates/<crate>/<version>` for the dates and the yanked flag, all read
on 2026-10-20.

### Advisories and their reach in Oxyn

| Advisory | Patched in | What it changes for Oxyn |
|---|---|---|
| RUSTSEC-2026-0346 (`rustls`) | >= 0.23.46 | The derived `Debug` of the TLS 1.2 and 1.3 client session values printed the master secret or the resumption PSK. Only a custom `ClientSessionStore` that formats them is affected; no Rust source under `crates/` or `drivers/` names one. Nothing to change in Oxyn |
| RUSTSEC-2026-0347 (`rustls`) | >= 0.23.46 | After an Encrypted Client Hello rejection, buffered application data went to the outer server. Only clients that set `EchMode::Enable` are affected; no source names `EchMode` |
| RUSTSEC-2026-0348 (`rustls`) | >= 0.23.46 | Brotli certificate decompression could allocate 1 GiB before authentication. Only the non-default `brotli` feature is affected: `cargo tree -e features -i rustls` shows `aws_lc_rs`, `std` and `tls12`, and the `brotli` crate in the lock comes from `tauri-codegen` |
| RUSTSEC-2026-0349 (`rustls`) | >= 0.23.46 | An unauthenticated ServerHello could leave later ClientHellos with a classical key share only, hence a connection without the hybrid post-quantum exchange. It reaches a client that keeps its resumption store, which is the rustls client default; the update needs no configuration |
| RUSTSEC-2026-0382 (`rustls-webpki`) | >= 0.103.16, < 0.104.0-alpha.1, or >= 0.104.0-alpha.8 | Certificate paths that contain the now-public emSign intermediate keys are rejected. Verification only gets stricter |
| RUSTSEC-2026-0381 (`webpki-roots`), RUSTSEC-2026-0383 (`webpki-root-certs`) | >= 1.0.10 | The root `emSign ECC Root CA - G3` leaves the bundled set. `webpki-roots` feeds `sqlx-core` (PostgreSQL) and `mysql_async` (MySQL); `webpki-root-certs` feeds `rustls-platform-verifier`, hence `reqwest` (`oxyn-llm`, `oxyn-embed`, `tauri-plugin-updater`). A server whose chain ends at that root no longer verifies, which is the intended result and leaves the verified-TLS policy of ADR-0052 as it was |
| RUSTSEC-2026-0353, -0354, -0355, -0356, -0357, -0358, -0359, -0361, -0362, -0363, -0364 (`wasmtime`) | >= 48.0.6, < 49.0.0 | Component-model and async host panics, a stack overflow when dropping a long chain of recursive types, the async-entrypoint requirement not enforced, handle tables charged to no limit, unaligned exception tags |
| RUSTSEC-2026-0378, -0379 (`wasmtime`) | >= 48.0.7, < 49.0.0 | A panic when lowering a host-created stream into a guest that expects another payload type; the Cranelift aarch64 backend could jump to a wrong target on a large `br_table` |

`wasmtime` is behind the `wasm-host` feature of `oxyn-plugin`, which is off by
default, which no manifest enables, and `oxyn-plugin` itself has no dependent in
the workspace (`cargo tree --all-features -i oxyn-plugin` prints only the crate):
the released application contains none of the thirteen. `--all-features`, which
`make lint`, `make test`, `make doc` and `make deny` all use, compiles and
audits it nonetheless, so the advisories fail the gate. 48.0.6 and 48.0.7 also
fix GHSAs outside this list (filesystem sandbox escape, wasi-http), and 48.0.7
updates its own `rustls` to 0.23.46. Sources: the RustSec advisory database,
`crates/<crate>/RUSTSEC-2026-<number>.md`, commit `e6f94f37`, read on
2026-10-20, with [RUSTSEC-2026-0382](https://rustsec.org/advisories/RUSTSEC-2026-0382)
as the entry published that day; release notes of
[rustls 0.23.46](https://github.com/rustls/rustls/releases/tag/v/0.23.46),
[rustls-webpki 0.103.16](https://github.com/rustls/webpki/releases/tag/v/0.103.16),
[webpki-roots 1.0.10](https://github.com/rustls/webpki-roots/releases/tag/v/1.0.10),
[Wasmtime 48.0.6](https://github.com/bytecodealliance/wasmtime/releases/tag/v48.0.6)
and [48.0.7](https://github.com/bytecodealliance/wasmtime/releases/tag/v48.0.7).

## Audit follow-up — checked on 2026-10-20

Facts read while applying the maintainer's decisions on the audit's open
questions. This section holds only what was read on 2026-10-20; rows elsewhere
in this file keep the date they were read.

### GitHub-hosted runners on a public repository

| Fact | Source | Checked on |
|---|---|---|
| "Use of the standard GitHub-hosted runners is free and unlimited on public repositories." In the page's table of standard runners for public repositories, `macos-latest` is a macOS runner with 3 CPUs (M1), 7 GB of RAM, 14 GB of SSD, arm64 | [GitHub-hosted runners reference](https://docs.github.com/en/actions/reference/runners/github-hosted-runners), raw HTML fetched with `curl -L` and read | 2026-10-20 |
| `so-keyldzn/oxyn` is public: `private` is `false` and `visibility` is `public` | `gh api repos/so-keyldzn/oxyn --jq '.private,.visibility'` | 2026-10-20 |

For pull requests, these rows supersede "Hence Linux only on PRs" (CI and
GitHub delivery, 2026-09-24) and "pull requests build Linux x86_64 only"
(sqlite-vec, 2026-10-06). Both stay true as the record of their own date: the
quota that caused the first was exhausted while the repository was private.
`qualite.yml` runs the macOS matrix on pull requests again, as decided by the
amendment of 2026-10-20 to
[ADR-0045](adr/0045-ci-selective-sur-les-pull-requests.md).

### Pinned manifest versions

`make versions` (`.claude/hooks/verifier_versions.py --manifests`) compares every
version pinned in `[workspace.dependencies]` of the root `Cargo.toml`, and in
`dependencies`, `devDependencies` and `packageManager` of
`apps/desktop/package.json`, with this file, one line at a time. On 2026-10-20 it
found 62 of the 131 pinned versions unrecorded: 18 crates and 44 npm packages.
They are recorded below, one package per row, which is the shape the check reads.

Each pin was read from its registry on 2026-10-20. A crate: the crates.io API
(`versions[].num` holds the pinned version, `crate.max_stable_version` is the
latest published, `versions[].created_at` its date). An npm package: the registry
document of the package (`versions` holds the pinned version, `dist-tags.latest`
is the latest published, `time` its date), and `npm view <package>@<pinned>
version` confirmed each of the 44. The pinned version exists in all 62 cases and
no crate is yanked. The *Latest published* column reads `same` when the pin is the latest,
otherwise the latest version and its publication date.

- A pin that is not the latest is recorded, not bumped. Raising one is a separate
  change, which reads the changelog first and moves the manifest and its row
  together ([`/versions`](../.claude/commands/versions.md)). The tables record the
  gap, they do not explain it; the registry facts that bear on some of the gaps
  follow.
- The `wasmtime` `48.0.1` row of the MSRV table (2026-09-05) is history: `48.0.5`,
  published on 2026-10-02, is what `Cargo.toml` pins. Its `rust-version` is
  `1.95.0` (crates.io API, 2026-10-20), the floor the `Cargo.toml` comment states.
  `50.0.0`, published on 2026-10-20, declares `1.97.0`.
- `rusqlite` `0.37.0` is not repeated here: it is already recorded as a
  deliberate pin, with its reason, in the MSRV table above, in the `Cargo.toml`
  comment and in [ADR-0010](adr/0010-contraintes-natives-sqlite.md).
- `@vitest/browser` and `@vitest/coverage-v8` `4.1.11` declare the exact peer
  `vitest` `4.1.11`, and `@storybook/addon-vitest@10.6.0` declares
  `vitest ^3.0.0 || ^4.0.0` (`npm view <package>@<version> peerDependencies`,
  2026-10-20): they stay on the 4 line with `vitest`, as the npm table above says.
- npm marks `eslint@9.39.5` as deprecated: "This version is no longer
  supported. Please see https://eslint.org/version-support for other options."
  (`npm view eslint@9.39.5 deprecated`, 2026-10-20). `@tanstack/eslint-config@0.4.0`
  declares the peer `eslint ^9.0.0 || ^10.0.0` and `eslint-plugin-storybook@10.6.0`
  declares `eslint >=8`. Recorded, not acted on.
- `@tanstack/react-start` is the one row of the npm table below that the check did
  not report: the TanStack row of the front-end table above already names it, but
  with two other packages on the same line. It has its own row here because a bump
  of the TanStack family changes several versions at once and this file records
  one package per row. On 2026-10-20 `pnpm audit --prod` (the `audit-npm` step of
  CI run `39470967933`) reports the critical advisory
  [GHSA-qx66-fv34-fjm8](https://github.com/advisories/GHSA-qx66-fv34-fjm8)
  against `@tanstack/react-start` `>=1.143.12 <1.168.60`, patched in `1.168.60`
  (published 2026-09-30), and against the transitive `@tanstack/start-server-core`
  `>=1.143.12 <1.169.39`. The pin `1.168.54` is inside the vulnerable range:
  recorded, not bumped here, and the bump moves the TanStack packages together.

Every pin the check reported was readable from its registry, so none sits under an
exemption.

#### Crates

| Crate | Pinned | Latest published | Registry | Checked on |
|---|---|---|---|---|
| `async-trait` | `0.1.92` | same | [crates.io API](https://crates.io/api/v1/crates/async-trait) | 2026-10-20 |
| `bitflags` | `2.13.1` | `2.13.2` (2026-09-10) | [crates.io API](https://crates.io/api/v1/crates/bitflags) | 2026-10-20 |
| `chrono` | `0.4.45` | same | [crates.io API](https://crates.io/api/v1/crates/chrono) | 2026-10-20 |
| `futures` | `0.3.34` | same | [crates.io API](https://crates.io/api/v1/crates/futures) | 2026-10-20 |
| `indexmap` | `2.14.2` | same | [crates.io API](https://crates.io/api/v1/crates/indexmap) | 2026-10-20 |
| `parking_lot` | `0.12.5` | same | [crates.io API](https://crates.io/api/v1/crates/parking_lot) | 2026-10-20 |
| `serde_json` | `1.0.151` | `1.0.154` (2026-10-11) | [crates.io API](https://crates.io/api/v1/crates/serde_json) | 2026-10-20 |
| `smallvec` | `1.16.0` | `1.16.3` (2026-10-10) | [crates.io API](https://crates.io/api/v1/crates/smallvec) | 2026-10-20 |
| `url` | `2.5.8` | same | [crates.io API](https://crates.io/api/v1/crates/url) | 2026-10-20 |
| `tempfile` | `3.27.0` | same | [crates.io API](https://crates.io/api/v1/crates/tempfile) | 2026-10-20 |
| `directories` | `6.0.0` | same | [crates.io API](https://crates.io/api/v1/crates/directories) | 2026-10-20 |
| `secrecy` | `0.10.3` | same | [crates.io API](https://crates.io/api/v1/crates/secrecy) | 2026-10-20 |
| `zeroize` | `1.9.0` | `1.9.1` (2026-10-06) | [crates.io API](https://crates.io/api/v1/crates/zeroize) | 2026-10-20 |
| `schemars` | `1.2.2` | `1.2.3` (2026-10-12) | [crates.io API](https://crates.io/api/v1/crates/schemars) | 2026-10-20 |
| `wasmtime` | `48.0.5` | `50.0.0` (2026-10-20) | [crates.io API](https://crates.io/api/v1/crates/wasmtime) | 2026-10-20 |
| `tracing-subscriber` | `0.3.23` | same | [crates.io API](https://crates.io/api/v1/crates/tracing-subscriber) | 2026-10-20 |
| `insta` | `1.48.0` | `1.49.0` (2026-10-03) | [crates.io API](https://crates.io/api/v1/crates/insta) | 2026-10-20 |
| `rstest` | `0.26.1` | `0.28.0` (2026-10-18) | [crates.io API](https://crates.io/api/v1/crates/rstest) | 2026-10-20 |

#### npm packages

| Package | Pinned | Latest published | Registry | Checked on |
|---|---|---|---|---|
| `@codemirror/autocomplete` | `6.20.3` | `6.20.4` (2026-10-19) | [npm registry](https://registry.npmjs.org/@codemirror%2Fautocomplete) | 2026-10-22 |
| `@codemirror/lang-json` | `6.0.2` | same | [npm registry](https://registry.npmjs.org/@codemirror%2Flang-json) | 2026-10-23 |
| `@codemirror/language` | `6.12.4` | `6.13.1` (2026-10-07) | [npm registry](https://registry.npmjs.org/@codemirror%2Flanguage) | 2026-10-20 |
| `@codemirror/search` | `6.7.2` | same | [npm registry](https://registry.npmjs.org/@codemirror%2Fsearch) | 2026-10-22 |
| `@codemirror/state` | `6.7.4` | `6.7.6` (2026-09-22) | [npm registry](https://registry.npmjs.org/@codemirror%2Fstate) | 2026-10-20 |
| `@codemirror/view` | `6.43.11` | `6.43.15` (2026-10-19) | [npm registry](https://registry.npmjs.org/@codemirror%2Fview) | 2026-10-20 |
| `@date-fns/tz` | `1.5.0` | same | [npm registry](https://registry.npmjs.org/@date-fns%2Ftz) | 2026-10-20 |
| `@fontsource-variable/geist` | `5.3.0` | same | [npm registry](https://registry.npmjs.org/@fontsource-variable%2Fgeist) | 2026-10-20 |
| `@fontsource-variable/geist-mono` | `5.3.0` | same | [npm registry](https://registry.npmjs.org/@fontsource-variable%2Fgeist-mono) | 2026-10-20 |
| `@lezer/highlight` | `1.2.3` | `1.2.5` (2026-09-28) | [npm registry](https://registry.npmjs.org/@lezer%2Fhighlight) | 2026-10-20 |
| `@shadcn/react` | `0.3.1` | same | [npm registry](https://registry.npmjs.org/@shadcn%2Freact) | 2026-10-20 |
| `@tanstack/react-form` | `1.33.5` | same | [npm registry](https://registry.npmjs.org/@tanstack%2Freact-form) | 2026-10-20 |
| `@tanstack/react-pacer` | `0.23.0` | `0.24.1` (2026-10-05) | [npm registry](https://registry.npmjs.org/@tanstack%2Freact-pacer) | 2026-10-20 |
| `@tanstack/react-router` | `1.170.41` | `1.170.45` (2026-10-16) | [npm registry](https://registry.npmjs.org/@tanstack%2Freact-router) | 2026-10-20 |
| `@tanstack/react-start` | `1.168.60` | `1.169.3` (2026-10-16) | [npm registry](https://registry.npmjs.org/@tanstack%2Freact-start) | 2026-10-20 |
| `@tanstack/react-virtual` | `3.14.13` | `3.14.21` (2026-10-16) | [npm registry](https://registry.npmjs.org/@tanstack%2Freact-virtual) | 2026-10-20 |
| `@tauri-apps/plugin-dialog` | `2.7.3` | `2.8.3` (2026-10-14) | [npm registry](https://registry.npmjs.org/@tauri-apps%2Fplugin-dialog) | 2026-10-20 |
| `class-variance-authority` | `0.7.1` | same | [npm registry](https://registry.npmjs.org/class-variance-authority) | 2026-10-20 |
| `cmdk` | `1.1.1` | same | [npm registry](https://registry.npmjs.org/cmdk) | 2026-10-20 |
| `cn` | `0.3.0` | `0.5.0` (2026-10-16) | [npm registry](https://registry.npmjs.org/cn) | 2026-10-20 |
| `date-fns` | `4.4.0` | same | [npm registry](https://registry.npmjs.org/date-fns) | 2026-10-20 |
| `embla-carousel-react` | `8.6.0` | same | [npm registry](https://registry.npmjs.org/embla-carousel-react) | 2026-10-20 |
| `input-otp` | `1.5.0` | same | [npm registry](https://registry.npmjs.org/input-otp) | 2026-10-20 |
| `react-day-picker` | `10.0.1` | `10.0.2` (2026-09-30) | [npm registry](https://registry.npmjs.org/react-day-picker) | 2026-10-20 |
| `react-resizable-panels` | `4.12.4` | `4.14.7` (2026-10-20) | [npm registry](https://registry.npmjs.org/react-resizable-panels) | 2026-10-20 |
| `recharts` | `3.8.0` | `3.10.1` (2026-07-25) | [npm registry](https://registry.npmjs.org/recharts) | 2026-10-20 |
| `tw-animate-css` | `1.4.0` | same | [npm registry](https://registry.npmjs.org/tw-animate-css) | 2026-10-20 |
| `@storybook/addon-a11y` | `10.6.2` | `10.6.3` (2026-10-20) | [npm registry](https://registry.npmjs.org/@storybook%2Faddon-a11y) | 2026-10-21 |
| `@storybook/addon-docs` | `10.6.2` | `10.6.3` (2026-10-20) | [npm registry](https://registry.npmjs.org/@storybook%2Faddon-docs) | 2026-10-21 |
| `@storybook/addon-vitest` | `10.6.2` | `10.6.3` (2026-10-20) | [npm registry](https://registry.npmjs.org/@storybook%2Faddon-vitest) | 2026-10-21 |
| `@tanstack/devtools-vite` | `0.8.5` | same | [npm registry](https://registry.npmjs.org/@tanstack%2Fdevtools-vite) | 2026-10-20 |
| `@tanstack/eslint-config` | `0.4.0` | same | [npm registry](https://registry.npmjs.org/@tanstack%2Feslint-config) | 2026-10-20 |
| `@tanstack/react-devtools` | `0.10.12` | `0.10.13` (2026-09-23) | [npm registry](https://registry.npmjs.org/@tanstack%2Freact-devtools) | 2026-10-20 |
| `@tanstack/react-query-devtools` | `5.102.8` | `5.104.5` (2026-10-19) | [npm registry](https://registry.npmjs.org/@tanstack%2Freact-query-devtools) | 2026-10-20 |
| `@tanstack/react-router-devtools` | `1.167.2` | same | [npm registry](https://registry.npmjs.org/@tanstack%2Freact-router-devtools) | 2026-10-20 |
| `@tanstack/router-plugin` | `1.168.42` | `1.168.47` (2026-10-16) | [npm registry](https://registry.npmjs.org/@tanstack%2Frouter-plugin) | 2026-10-20 |
| `@testing-library/dom` | `10.4.2` | same | [npm registry](https://registry.npmjs.org/@testing-library%2Fdom) | 2026-10-20 |
| `@testing-library/react` | `16.3.3` | same | [npm registry](https://registry.npmjs.org/@testing-library%2Freact) | 2026-10-20 |
| `@types/node` | `26.5.1` | `26.6.5` (2026-10-09) | [npm registry](https://registry.npmjs.org/@types%2Fnode) | 2026-10-20 |
| `@types/react` | `19.3.0` | same | [npm registry](https://registry.npmjs.org/@types%2Freact) | 2026-10-20 |
| `@types/react-dom` | `19.3.0` | same | [npm registry](https://registry.npmjs.org/@types%2Freact-dom) | 2026-10-20 |
| `@vitest/browser` | `4.1.11` | `5.0.5` (2026-10-15) | [npm registry](https://registry.npmjs.org/@vitest%2Fbrowser) | 2026-10-20 |
| `@vitest/coverage-v8` | `4.1.11` | `5.0.5` (2026-10-15) | [npm registry](https://registry.npmjs.org/@vitest%2Fcoverage-v8) | 2026-10-20 |
| `eslint` | `9.39.5` | `10.13.0` (2026-10-16) | [npm registry](https://registry.npmjs.org/eslint) | 2026-10-20 |
| `eslint-plugin-storybook` | `10.6.2` | `10.6.3` (2026-10-20) | [npm registry](https://registry.npmjs.org/eslint-plugin-storybook) | 2026-10-21 |
| `jsdom` | `30.0.1` | `30.1.3` (2026-10-15) | [npm registry](https://registry.npmjs.org/jsdom) | 2026-10-20 |
| `prettier` | `3.9.6` | `3.9.10` (2026-10-10) | [npm registry](https://registry.npmjs.org/prettier) | 2026-10-20 |
| `prettier-plugin-tailwindcss` | `0.8.1` | same | [npm registry](https://registry.npmjs.org/prettier-plugin-tailwindcss) | 2026-10-20 |

## Vite and the launch-editor advisories — checked on 2026-10-21

The `vite` pin of `apps/desktop` moved from `8.3.0` to `8.3.3`. This section
supersedes the `vite` value cited in the interface table above (`8.3.0`) and in
the Node-engines row (`vite 8.3.0`): the `engines` field of vite 8.3.3 was not
re-read, the Node line the CI takes (22) is unchanged.

| Fact | Source | Checked on |
|---|---|---|
| Three advisories published on 2026-10-20 affect the `launch-editor` copy bundled in `vite`: [GHSA-587h-7wmj-h5pm](https://github.com/advisories/GHSA-587h-7wmj-h5pm) (CVE-2026-112942, **high**, 19:48:25Z, `fs.existsSync()` check bypassed on Windows), [GHSA-698v-4r3x-3vcv](https://github.com/advisories/GHSA-698v-4r3x-3vcv) (CVE-2026-112943, moderate, 18:30:24Z, `%` not escaped by `escapeCmdArgs()` on Windows) and [GHSA-r825-w8mm-5pxw](https://github.com/advisories/GHSA-r825-w8mm-5pxw) (CVE-2026-112944, moderate, 19:51:52Z, editor CLI flag misinterpretation). For the 8.3 line the vulnerable range is `>= 8.3.0, <= 8.3.2` and the first patched version is `8.3.3` | `gh api /advisories/<id>` | 2026-10-21 |
| `vite` `8.3.3` was published on 2026-10-06T04:10:19Z, `8.3.4` on 2026-10-08T12:07:04Z and `8.3.5`, the `latest` dist-tag, on 2026-10-21T06:38:54Z. `8.3.3` is the smallest patched version of the line, hence the pin; a later bump goes through the usual update flow | `npm view vite time dist-tags --json` | 2026-10-21 |
| With `vite` `8.3.3` the lockfile also moves `rolldown` and its bindings from `1.2.8` to `1.2.14` and `@oxc-project/types` from `0.149.0` to `0.154.0`: they are vite's own dependencies, not pins of the application | `pnpm install --lockfile-only` in `apps/desktop`, diff of `pnpm-lock.yaml` | 2026-10-21 |
| On this change `pnpm audit --prod` reports "No known vulnerabilities found", `make front-controles` passes (Prettier, ESLint, `tsc --noEmit`, licenses, audit) and `pnpm build` completes with the SPA prerender of `/` | commands run in `apps/desktop` | 2026-10-21 |

## Elasticsearch and OpenSearch — checked on 2026-10-22

For [ADR-0062](adr/0062-elasticsearch-opensearch-rest-json.md).

| Fact | Source | Checked on |
|---|---|---|
| The workspace reuses `reqwest` **0.13.4**, already pinned with `json`, `stream`, `rustls`, `http2`, `charset` and `system-proxy`; `Cargo.lock` contains one resolved reqwest version. The driver calls `no_proxy()` and disables redirects, so a configured database endpoint cannot send an authorization header through a system proxy or a redirect target. | `Cargo.toml`, `Cargo.lock`, [reqwest 0.13.4 ClientBuilder](https://docs.rs/reqwest/0.13.4/reqwest/struct.ClientBuilder.html) | 2026-10-22 |
| Elasticsearch documents that deep `from`/`size` pagination is limited by `index.max_result_window` (10,000 by default), and recommends `search_after`; a point in time keeps order stable across its repeated searches and provides `_shard_doc` as a tie breaker. | [Elasticsearch pagination](https://www.elastic.co/docs/reference/elasticsearch/rest-apis/paginate-search-results) | 2026-10-22 |
| Elasticsearch REST compatibility is only across one major version: 9.x can honor 8.x requests and responses with `compatible-with=8`. Elastic's Docker documentation names **9.5.5** as the current exact image tag. | [Elasticsearch API compatibility](https://www.elastic.co/docs/reference/elasticsearch/rest-apis/compatibility), [Docker installation](https://www.elastic.co/docs/deploy-manage/deploy/self-managed/install-elasticsearch-docker-basic) | 2026-10-22 |
| OpenSearch exposes `POST /_tasks/_cancel` and `POST /_tasks/{task_id}/_cancel`; not every task is cancellable, and the response reports `cancellable` and `cancelled`. The official image tag **3.0.0** exists for both amd64 and arm64. | [OpenSearch cancel tasks](https://docs.opensearch.org/latest/api-reference/tasks/cancel-tasks/), [OpenSearch Docker tag 3.0.0](https://hub.docker.com/r/opensearchproject/opensearch/tags?name=3.0.0&ordering=last_updated&page=1) | 2026-10-22 |
| Docker Hub's most recent tags are `elasticsearch:9.5.5` (updated 2026-10-23), the image the CI `serveurs` job runs, and `opensearchproject/opensearch:3.9.0` (updated 2026-09-29), supersedes 3.0.0 above as the release check. Both ran the 11 integration tests of the driver green on 2026-10-24. | [Docker Hub elasticsearch](https://hub.docker.com/_/elasticsearch/tags), [Docker Hub opensearch](https://hub.docker.com/r/opensearchproject/opensearch/tags), `cargo nextest run -p oxyn-driver-elasticsearch --run-ignored all` | 2026-10-24 |
| A point in time is opened by `POST /<index>/_pit` and its identifier read from `id` on Elasticsearch, by `POST /<index>/_search/point_in_time` and `pit_id` on OpenSearch, which has no `_pit` route; both continue with `search_after` and `pit.id` in the body. | integration test `a_point_in_time_stream_that_would_not_fit_stays_under_a_memory_bound` against both images above | 2026-10-24 |
| The driver reads hits through `serde_json`'s `raw_value` feature (`RawValue`), so a number keeps its exact text: an `unsigned_long` beyond 2^53 does not pass through `f64`. The feature adds no dependency. | [serde_json features](https://docs.rs/crate/serde_json/latest/features), `Cargo.lock` | 2026-10-24 |
| The shadcn `@svgl` registry (670 entries) has no Elasticsearch, Elastic or OpenSearch mark: the connection tile wears the generic database icon. | [api.svgl.app](https://api.svgl.app), the index the registry serves | 2026-10-24 |
| An index name "cannot start with `-`, `_`, or `+`" and "cannot be `.` or `..`"; it cannot hold `\` or `/`. The classifier rests on the first rule: the first path segment starting with `_` is the API. `_all` is the one target that starts with `_`. | [Elasticsearch create index API](https://www.elastic.co/docs/api/doc/elasticsearch/operation/operation-indices-create) | 2026-10-24 |
| A search returns **10** hits when it names no `size`: a point-in-time page shorter than its `size`, or than 10, is the last one. | [Elasticsearch search API](https://www.elastic.co/docs/api/doc/elasticsearch/operation/operation-search) | 2026-10-24 |
| Elasticsearch has searched `dense_vector` fields by approximate kNN since **8.0**, the driver's floor: every Elasticsearch session declares `VECTOR_SEARCH`. OpenSearch does so through its `opensearch-knn` plugin, bundled in the full distribution and absent from the minimal one; the driver reads `GET /_cat/plugins`, which lists it on the 3.9.0 image. | [Elasticsearch 8.0 kNN search](https://www.elastic.co/guide/en/elasticsearch/reference/8.0/knn-search.html), [OpenSearch plugins](https://docs.opensearch.org/latest/install-and-configure/plugins/), `GET /_cat/plugins` on `opensearchproject/opensearch:3.9.0` | 2026-10-24 |
| On macOS 26 (Darwin 25), a process that allocates and frees a 5 MB buffer sixty times holds 61 `MALLOC_LARGE` regions, about 290 MB dirty, by `footprint`: freed large allocations stay resident. The stream test therefore asserts its resident-memory bound on Linux, where the CI runs it. | a probe program outside the repository, `/usr/bin/footprint -p` | 2026-10-24 |
## ClickHouse protocol and client libraries — checked on 2026-10-23

For [ADR-0060](adr/0060-clickhouse-driver-over-http-rowbinary.md). The driver
adds no dependency: it uses the workspace's `reqwest` 0.13.4 (`rustls`),
`arrow` 59.3.0, `uuid`, `url` and `secrecy`.

| Fact | Consequence | Source |
|---|---|---|
| Docker Hub `clickhouse/clickhouse-server` tags updated on 2026-10-22/23: `26.9.25.15` (and `latest`), `26.3.59.10`, `25.8.33.6`; the `.3` and `.8` lines are the long-term ones still patched | the driver and the CI service run `26.3.59.10` | [Docker Hub tags API](https://hub.docker.com/v2/repositories/clickhouse/clickhouse-server/tags) |
| `clickhouse` 0.15.3 (official, HTTP, updated 2026-10-13): rows decoded through `serde` into types known at compile time, "`Dynamic` data type is not supported for now", TLS off by default behind `native-tls` / `rustls-tls*` features, default feature `lz4` | no untyped result for arbitrary SQL | crates.io API; [docs.rs](https://docs.rs/clickhouse/0.15.3/clickhouse/) |
| `clickhouse-arrow` 0.2.1 (native protocol, updated 2025-12-14) depends on `arrow ^57` and `webpki-roots ^1`; `klickhouse` 0.16.0 (native, 2026-10-12) has its own value model; `clickhouse-rs` 0.1.21 was last updated 2023-09-10 | none fits the workspace's Arrow 59 and platform verifier | crates.io API, `/dependencies` |
| `FORMAT Arrow`/`ArrowStream` output: `DateTime` → `UINT32`, `Date32` → `UINT16`, `Enum8`/`Enum16` → `INT8`/`INT16`, `Int128`/`UInt128`/`Int256`/`UInt256` and `IPv6` → `FIXED_SIZE_BINARY`, `IPv4` → `UINT32`; types with no Arrow equivalent (`JSON`, `Dynamic`, `AggregateFunction`) follow `output_format_arrow_unsupported_types`, default `binary` | zone, labels and wide integers would need rewriting after decoding | [Arrow format](https://clickhouse.com/docs/interfaces/formats/Arrow) |
| `SELECT 1 -- x\r, 2` and `SELECT 1 # x\r, 2` return one column, `SELECT 1 -- x\n, 2` two; `SELECT $t$a;b$t$, $$c;d$$` returns `a;b` and `c;d`; `SELECT /* a /* b */ c */ 1` returns `1` | line comments end at `\n` only; heredocs are strings; block comments nest | observed on 26.3.59.10 |
| `` SELECT 1 AS `a\`b` `` names the column `` a`b ``; `` `c``d` `` names it `` c`d `` | a backslash escapes inside a backtick identifier, doubling too | observed on 26.3.59.10 |
| `SELECT 1; SELECT 2` over HTTP: `Code: 62 … Syntax error (Multi-statements are not allowed)` | one statement per request, proven by the server | observed on 26.3.59.10 |
| A `param_v` value is read with TSV escapes: `a\b` binds as `0x61 0x08`, `a\\b` as `a\b`, `a\tb` as a tab; a raw tab or line feed fails with `Code: 457 BAD_QUERY_PARAMETER`. The `database` URL parameter is taken as is (`d\b` stays `d\b`) | parameter values are escaped by the driver | observed on 26.3.59.10 |
| `readonly = 1` on a request refuses writes, DDL, `url()` and a statement's own `SETTINGS` clause (`Cannot modify 'max_threads' setting in readonly mode`, code 164); settings sent before `readonly` in the URL are applied | read-only reads carry no `SETTINGS` clause | observed on 26.3.59.10 |
| `KILL QUERY WHERE query_id = {id:String} TEST` matching nothing answers 200, an empty body, no `X-ClickHouse-Format` | the kill right is read from the status | observed on 26.3.59.10 |
| `KILL QUERY … ASYNC` on a read whose client stopped reading answered after 28 s, when the reading pipe closed; the read then left `system.processes` at once. A read whose connection closes leaves it about 10 s later without a kill | the driver drains or closes the response while it kills, never after | observed on 26.3.59.10 (`curl … \| sleep 30`) |
| A 200 response carries `X-ClickHouse-Exception-Tag`; an exception raised after rows left is appended to the body as `\r\n__exception__\r\n<tag>\r\n<message>\n<length> <tag>\r\n__exception__\r\n` | the decoder cuts the body at the tag's marker | observed on 26.3.59.10 |
| `ALTER TABLE t RENAME TO u` fails (`Expected COLUMN`, code 62); `RENAME TABLE db.t TO db.u` works; `DROP MATERIALIZED VIEW mv` fails (code 62), `DROP VIEW mv` drops it; `TRUNCATE TABLE` runs on a `MergeTree` table | the catalog's rename and drop use ClickHouse's forms; `TRUNCATE` is declared | observed on 26.3.59.10 |
| `cancel_http_readonly_queries_on_client_close = 1` stops an aggregate whose connection closed only when the request runs at `readonly` 1 or 2; at 0 it ran on (still there 22 s later). `readonly = 2` lets a `SETTINGS` clause, `url()`, `remote()` and `file()` through | a read not bounded read-only carries `readonly = 2` | observed on 26.3.59.10 |
| Every response carries `X-ClickHouse-Query-Id` with the id sent, except a request refused before it ran (authentication 516, unknown database 81, bad setting 27), which carries `X-ClickHouse-Exception-Code` | an answer without either is not taken for the server's; redirects are not followed | observed on 26.3.59.10 |
| `throwIf` raised at row 150,000 of a `RowBinaryWithNamesAndTypes` read answers 200 with the exception in the body; with `wait_end_of_query = 1` it answers 500. The version that introduced `X-ClickHouse-Exception-Tag` was not found in the changelog or the documentation searched on 2026-10-23 | a server whose 200 carries no tag is asked `wait_end_of_query = 1`, or refused when the account takes no setting; the check is the header, not a version | observed on 26.3.59.10 |
| `send_progress_in_http_headers` defaults to 0, `output_format_binary_encode_types_in_binary_format` to 0, `output_format_binary_write_json_as_string` to 0 (`system.settings`) | the driver pins the three and refuses a statement's own `SETTINGS` that changes them | observed on 26.3.59.10 |
| A dictionary is a row of `system.tables` with engine `Dictionary`, readable by `SELECT`; `RENAME TABLE` renames it, `DROP TABLE` refuses it (code 520, `use DROP DICTIONARY`), `TRUNCATE` refuses it (code 62) | dictionaries are listed as tables; the server refuses the table forms that do not apply | observed on 26.3.59.10 |
| `create_table_query` shows a dictionary source's `PASSWORD` and an `S3` engine's secret key as `'[HIDDEN]'`; `format_display_secrets_in_show_and_select` is `false` by default and only takes effect when the server's configuration allows it | the driver's definition read sets it to `0` whenever the account may carry settings, and withholds the definition when it still reads `1` | observed on 26.3.59.10 |
| `sleepEachRow` refuses more than 3 s per block (`Code: 160 TOO_SLOW`); `max_execution_time` stops a read with code 159 | the timeout test reads one-row blocks | observed on 26.3.59.10 |
| `errorCodeToName` returns, for each of the 54 codes the classification names, the name the driver gives it (8 `THERE_IS_NO_COLUMN` … 394 `QUERY_WAS_CANCELLED`, 497 `ACCESS_DENIED`, 516 `AUTHENTICATION_FAILED`, 999 `KEEPER_EXCEPTION`) | the classification's codes are the server's | one query over the whole list, on 26.3.59.10 |
