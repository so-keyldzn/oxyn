---
description: Add or modify a screen of the Tauri interface
argument-hint: "<the screen or component, e.g. history panel>"
allowed-tools: Bash, Read, Write, Edit, Grep, Glob, WebFetch, Skill
---

Purpose: add or modify **$ARGUMENTS** in the Tauri interface.

## Before writing

1. **Read [.claude/rules/front.md](../rules/front.md) with `Read`**, even if the
   file to create does not exist yet: a `paths:` rule only loads when a matching
   file is read, and otherwise the first file of an area is written without it.
2. [UX-SPEC](../../docs/UX-SPEC.md) — behaviors and states are authoritative.
3. [ARCHITECTURE § 2 bis](../../docs/ARCHITECTURE.md#2-bis-linterface-tauri) — the
   IPC bridge and what does not cross it.
4. [FIGMA-HANDOFF](../../docs/FIGMA-HANDOFF.md) — the board, if it exists.

## Tools rather than memory

| For | Use |
|---|---|
| adding or composing a shadcn component on Base UI | the `shadcn` skill, then `pnpm exec shadcn add` |
| routes, loading, query state | the `tanstack-router-best-practices`, `tanstack-query` skills |
| command, channel, webview capabilities | the `tauri-v2` skill, then the official page — [I-12](../../CLAUDE.md#i-12) |
| reading a board | the Figma MCP (`get_design_context`, `get_screenshot`) |

## The order that avoids the rewrite

**1. The `Command` first.** If none expresses the action, that is what is
missing: [`/commande`](commande.md). A screen that finds a shortcut to the
store or to a driver creates the second path that [I-01](../../CLAUDE.md#i-01)
forbids.

**2. The Tauri command, `async`, that emits this `Command`.** Then the
TypeScript mirror of what crosses, **in the same commit**. A Tauri command
widens what a script in the webview can do: [`/securite`](securite.md).

**3. The component, data and callbacks through props, and one story per
state** — initial, in progress, populated, **empty**, error. Empty is the one
people forget, and it is the first one a new user sees.

**4. The feature that connects** the component to `src/lib/ipc`. It is the only
layer that talks to the backend; that is why stories never need it.

The reverse order — the screen first, the backend "plugged in later" — produces
a component whose props fit a mock, and an IPC mirror written for it.

## The traps specific to this move

The forbidden constructs live in [front.md](../rules/front.md); what follows is
what goes wrong **in the sequence**.

**The mirror updated on one side only.** Rust renames `rows` to `cells`, the
TypeScript still reads `rows`: the grid shows "empty" on a populated result.
Neither `tsc` nor `cargo` sees it.

**The error state that paraphrases.** The audience reads PostgreSQL messages:
the server's message, code included, is displayed as `IpcError` carries it.

**The connection identifier in a story or a title.** A story is published with
Storybook; [I-03](../../CLAUDE.md#i-03) does not stop at logs.

**The screen written for PostgreSQL.** It assumes a schema and SQL, and does not
exist for a driver that has neither: the interface is conditioned on
capabilities from now on ([ADR-0003](../../docs/adr/0003-driver-capabilities.md)).

## Verify

```bash
make front          # format, lint, types, stories (axe included), build
make desktop-dev    # the real window, on a temporary workspace
make qualite
```

Then [revue-ui](../checklists/revue-ui.md), and `relecteur-securite` if a Tauri
command was added.
