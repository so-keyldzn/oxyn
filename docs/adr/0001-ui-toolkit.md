# ADR-0001 — UI toolkit: GPUI, with strict isolation

**Status:** superseded · **Date:** 2026-09-05
**Superseded by:** [ADR-0029](0029-interface-tauri-shadcn.md), on 2026-09-15. What
follows is kept as it was decided: it is the reason why GPUI looked like the right
choice, and what changed since can be read in ADR-0029.

## Context
"Native first, blazing fast" rules out Electron and Tauri. In pure Rust, two credible
options: egui (immediate mode, stable, cross-platform) and GPUI (chosen, the engine of a
code editor, excellent at text). Oxyn's two critical surfaces are the query editor and
the results grid.

## Decision
Adopt **GPUI**. Forbid any dependency on the toolkit outside `oxyn-ui` and `oxyn-app`.

> **Clarified by [ADR-0009](0009-source-dependance-gpui.md).** The initial wording
> spoke of "pinning a precise commit", assuming the upstream monorepo would have to be
> vendored. That is wrong: `gpui` is published on crates.io. Consuming it from the
> registry, at an exact version, is settled by ADR-0009.

## Rationale
The dominant cost of the project is a professional-grade code editor. GPUI provides a
proven text engine and editing primitives; egui would force us to rebuild them, which
exceeds the cost of GPUI's API instability.

## Consequences
* **+** Text rendering and latency of a native code editor; familiar flexbox layout.
* **+** *Checked on 2026-09-05*: a GPUI 0.2.2 window using `Application::new().run`,
  `cx.open_window` and `impl Render` compiles on macOS 26.2 / arm64 / Rust 1.89 in
  1 min 52. The doubt about the very feasibility of the dependency is lifted.
* **−** Poor documentation, reading upstream code often necessary; spaced-out
  releases on crates.io (see ADR-0009).
* **−** Windows is fragile → macOS and Linux first, deliberately.
* The isolation rule keeps the decision reversible: switching to egui remains a
  rewrite of two crates until the end of phase 1.

## Rejected alternatives
* **egui** — kept as a fallback plan, not as a target.
* **Iced** — good model, text engine insufficient for a code editor.
* **Slint / Dioxus** — DSL or VDOM; no decisive advantage here.
