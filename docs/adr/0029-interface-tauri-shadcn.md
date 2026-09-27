# ADR-0029 — Web interface in Tauri: TanStack Start, shadcn/ui on Base UI

**Status:** accepted · **Date:** 2026-09-15
**Supersedes:** [ADR-0001](0001-ui-toolkit.md) on the choice of toolkit, and
[ADR-0009](0009-source-dependance-gpui.md) once the GPUI crates are removed. The
isolation rule of ADR-0001 remains in force, transposed to Tauri.

## Context

ADR-0001 ruled out Tauri in the name of "native first". The dominant cost it
named — a professional-quality code editor and results grid — was confirmed:
`oxyn-ui` and `oxyn-app` total **~14,500 lines** on 2026-09-15, of which 1,686
for the grid alone and 1,245 for the editor, without reaching the level of
finish of the Figma mockup. Every component (field, menu, dialog, tree,
tooltip, keyboard focus) is written there by hand, and GPUI has no component
library nor documentation
([ADR-0001](0001-ui-toolkit.md#consequences)).

The web ecosystem provides these components ready-made, accessible, and
testable in isolation. The facts, checked against the registry on 2026-09-15
([RESEARCH-NOTES](../RESEARCH-NOTES.md#tauri-interface-and-front-end)):

* `tauri 2.11.5` on crates.io, `rust-version = 1.77.2`, compatible with the
  pinned toolchain `1.98.1`;
* Tauri only serves static files: SSG, SPA or MPA, no SSR;
* `@tanstack/react-start 1.168.54` has an SPA mode that produces a static shell;
* `shadcn 4.21.0` generates a TanStack Start project on Base UI
  (`shadcn create -t start -b base`);
* `@storybook/addon-vitest 10.6.0` requires `vitest ^3 || ^4`: the 5 series
  (5.0.0 published on 2026-09-03) cannot be used with Storybook.

The rest of the repository does not know the interface: the eleven other
crates (~45,000 lines) do not depend on `gpui` ([I-08](../../CLAUDE.md#i-08)).
The switch only touches the two crates ADR-0001 had isolated for that purpose.

## Decision

**Oxyn's interface is a web application served by Tauri 2.**

| Layer | Choice | Where |
|---|---|---|
| Native host | `tauri` 2, window and IPC | crate `crates/oxyn-desktop`, binary `oxyn-desktop` |
| Front-end framework | TanStack Start in **SPA mode**: file-based routing, loaders | `apps/desktop` |
| Front-end data | TanStack Query (IPC calls), Table and Virtual (grid), Form, Store, Hotkeys, Pacer | `apps/desktop` |
| Components | shadcn/ui, `base-nova` style, **Base UI** primitives | `apps/desktop/src/components/ui` |
| Style | Tailwind CSS 4, tokens as CSS variables | `apps/desktop/src/styles.css` |
| Component workshop | Storybook 10 with `addon-vitest` and `addon-a11y` | `apps/desktop/.storybook` |
| Package manager | `pnpm`, version pinned by `packageManager` | `apps/desktop/package.json` |

**TanStack Start never runs as a server.** No SSR, no *server functions*, no
*server routes*: in Tauri there is no HTTP server, and the only backend is
Rust. Everything a *server function* would do goes through a Tauri command.

**A Tauri command only emits a `Command`.** `crates/oxyn-desktop` receives an
IPC call, translates it into an `oxyn_core::Command` carrying `Actor::Human`,
and hands it to the `Executor`. No Tauri command calls a driver, the keychain
or the store directly: that would be the second execution path
[I-01](../../CLAUDE.md#i-01) forbids, this time exposed to JavaScript.

**Results cross the IPC as pages of formatted cells.** The front end requests
a bounded window of rows (`result_page`, at most 2,000 rows);
`crates/oxyn-desktop` reads it from the `ResultBuffer` and formats each cell
with `oxyn_data::format_cell`, which relies on the `arrow-rs` formatters
**also used by the export** and by the GPUI grid. The front end does not
decode Arrow and reformats nothing: a timestamp or a binary rendered in
JavaScript would diverge from the exported file, which
[UX-SPEC](../UX-SPEC.md#what-is-exported-is-what-is-displayed) forbids. It
never holds the whole result: the virtualized grid only keeps the visible
pages ([I-06](../../CLAUDE.md#i-06), [ADR-0002](0002-arrow-result-model.md)).

**Isolation rule, transposed.** No crate outside `oxyn-desktop` depends on
`tauri`. Until the GPUI crates are removed, the old rule still holds for
`gpui`.

**The migration is done by parity.** `oxyn-ui` and `oxyn-app` stay in the
workspace and compile until `apps/desktop` covers the same screens
([IMPLEMENTATION-PLAN](../IMPLEMENTATION-PLAN.md)). The `oxyn-desktop` binary
carries its own assembly of the backend: sharing it would require moving
`ConnectionDraft` out of `oxyn-ui`, work thrown away when the crate is removed.

## Consequences

* **+** Accessible components (keyboard, focus, ARIA) come from Base UI instead
  of being rewritten; accessibility stops being the structural risk ADR-0001
  named.
* **+** Each component is developed and tested in isolation in Storybook,
  including its five states ([UX-SPEC](../UX-SPEC.md#states-of-a-view)).
* **+** Windows stops being "fragile": WebView2 is Tauri's primary target.
* **+** The Figma mockup translates into existing components, with no rendering
  engine to write.
* **−** **"Native first — no webview, no JS runtime" is abandoned.**
  It is a principle of the [VISION](../VISION.md), fixed in the same commit.
* **−** Three web engines: WKWebView (macOS), WebView2 (Windows), WebKitGTK
  (Linux). A rendering checked on one is not checked on the others.
* **−** The budgets of [PERFORMANCE](../PERFORMANCE.md) — 8 ms p99 frame while
  scrolling, 1 s cold start — are **no longer acquired by construction**: they
  are measured again in the webview. Each grid page pays an IPC crossing and a
  JSON serialization of formatted cells.
* **−** New input surface: a cell's content, a catalog object name or a model
  response rendered in the DOM. An XSS in the webview reaches the Tauri
  commands. Hence: strict CSP, minimal Tauri *capabilities*, no
  `dangerouslySetInnerHTML` on received data
  ([SECURITY](../SECURITY.md#input-surface)).
* **−** Two toolchains: `cargo` and `pnpm`. `make qualite` must cover both,
  otherwise the front end escapes the gate.
* **−** During the migration, two interfaces coexist and the backend is
  assembled twice.

**Exit cost:** rewrite `apps/desktop` and `crates/oxyn-desktop`. The rest of
the product knows neither Tauri nor React. This cost grows with the number of
screens; it is bounded by the isolation rule, which must hold.

**Reconsider if** the grid does not hold **8 ms p99** while scrolling a
one-million-row result on macOS and Linux, measured in the production webview;
or if WebKitGTK makes the application unusable on Linux.

## Rejected alternatives

| Alternative | Reason for rejection |
|---|---|
| Stay on GPUI | the cost of each hand-written component exceeds that of the webview, and the gap with the mockup does not close |
| egui | same problem as GPUI, with an editor to build on top ([ADR-0001](0001-ui-toolkit.md)) |
| Electron | ships Chromium and Node in every package, and a JS runtime on the backend side while the backend is Rust |
| TanStack Start with SSR or *server functions* | Tauri only serves static files; an embedded Node server would duplicate the backend |
| shadcn/ui on Radix | Base UI chosen as a product choice; `shadcn` supports both, and the repository's `shadcn` skill lists the API differences (`render` and not `asChild`) |
| Vitest 5 | `@storybook/addon-vitest 10.6.0` only accepts `^3 \|\| ^4` |
| Arrow IPC decoded in the front end (`apache-arrow`) | formatting a cell would happen twice, in Rust for export and in JavaScript for the screen, and the two would diverge; a page of 2,000 formatted rows stays small |
| TypeScript 7 | the skeleton generated by `shadcn 4.21.0` pins `typescript ^6`; moving to 7 is a separate decision, to check against ESLint and Storybook's docgen |
