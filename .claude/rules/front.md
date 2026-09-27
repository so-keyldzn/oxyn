---
paths:
  - "apps/desktop/**"
  - "crates/oxyn-desktop/**"
---

# Tauri interface — conventions

The decision and its reasons live in
[ADR-0029](../../docs/adr/0029-interface-tauri-shadcn.md); the IPC bridge in
[ARCHITECTURE](../../docs/ARCHITECTURE.md#2-bis-the-tauri-interface); behaviors in
[UX-SPEC](../../docs/UX-SPEC.md). This rule carries what gets missed when
writing code here.

## A single path to the backend

`invoke` is only called by `call`, in `src/lib/ipc/client.ts`; the modules of a
domain (`src/lib/ipc/<domain>.ts`) go through `call`. A view that calls it
itself creates the second path that [I-01](../../CLAUDE.md#i-01) forbids, and it
is the one an XSS would take. A feature starts with a Tauri command in
`crates/oxyn-desktop/src/commands.rs` **that emits a `Command`** — never with a
direct call to the store, the keychain or a driver.

A domain lives in its modules: `src/backend/<domain>.rs` (the `impl Backend`),
`src/commands/<domain>.rs` (the `#[tauri::command]`s), `src/ipc/<domain>.rs`
(what crosses), declared by one `mod` line and one block in
`generate_handler!`. On the front-end side, `src/lib/ipc/<domain>.ts` and
`src/features/<domain>/`.

Adding a Tauri command widens what a script in the webview can do:
[`/securite`](../commands/securite.md) before merging.

**A Tauri command without `async` runs on the main thread**
([RESEARCH-NOTES](../../docs/RESEARCH-NOTES.md#tauri-interface-and-front-end)). A body
that reads the store, the keychain or a batch spilled to disk freezes the
window there ([I-05](../../CLAUDE.md#i-05)) — and nothing reports it, since the
read is fast on the dev machine. `async fn`, or `#[tauri::command(async)]`;
synchronous only for state already in memory. `code_interdit.py` asks the
question for every synchronous command written.

`src/lib/ipc/types.ts` is the mirror of `src/ipc.rs`, and every
`src/lib/ipc/<domain>.ts` that of `src/ipc/<domain>.rs`. A field renamed on one
side only shows up at run time: both change in the same commit.

## The mirror is a schema, not a type

`call` takes a **schema** and parses the response; `invoke<T>` only casts
([ADR-0031](../../docs/adr/0031-validation-des-reponses-ipc.md)). What follows
when writing:

- a boundary type is declared **once**, as a schema, and its type follows:
  `export const X = z.object({…})` then `export type X = z.infer<typeof X>`.
  Writing an `interface` next to a schema recreates the mirror that was just
  removed;
- a Rust `Option<T>` is `.nullable()`, **never** `.optional()`: no
  `skip_serializing_if` exists on the `oxyn-desktop` side, so a `None` comes out
  as `"field": null` and the key is always there;
- the tag of a union is not always `type` — `RunTarget` and `DestinationChoice`
  tag on `kind`, `FacetFreshness` on `state`. A `z.discriminatedUnion` on the
  wrong tag fails on **every** response, not on a rare case;
- a newtype variant is **flattened** by serde: the pattern is
  `Inner.extend({ type: z.literal("…") })`, not a nested field;
- a field that Rust carries as `&'static str` **with no enum behind it** is not
  validated with `z.enum`: it would fail a valid response the day a value is
  added. Two cases, depending on what the Rust guarantees:

  | The Rust | The schema | Why |
  |---|---|---|
  | produces an **open** set (a preset identifier, an error class) | `z.string()`, the *type* narrowed | an unknown value is legitimate and must pass |
  | produces a set **closed by a `match` with a catch-all arm** (`_ => "unknown"`) | `z.enum([…]).catch("unknown")` | the type stays usable by a `switch` or a `Record`, and the unknown degrades **one badge** instead of failing the whole response |

  **A `.catch` assumes a value that expresses ignorance.** `unknown`, `other`:
  words whose meaning is "I could not read it". Where the type has none —
  `ProviderKind` only has protocol names —, any fallback is a **false
  statement**, and the field is validated strictly. The question is not "is
  failing annoying?" but "is there an honest value?". The decisive case is
  `AgentProvenance.kind`, which *signs* what a conversation proposes
  ([ADR-0023](../../docs/adr/0023-fournisseurs-declares-et-provenance.md)): a
  provenance that fails costs one proposal, a provenance that lies costs the
  whole mechanism. If resilience becomes necessary there, it is obtained by
  adding an ignorance variant **on the Rust side**, like `Ending::Unknown` —
  never by inventing it in the front end;
- a `u64` **that the server reports** (`estimatedRows`, `sizeBytes`) is
  validated without `.int()`: zod tests `Number.isSafeInteger` there, and beyond
  2^53 it would reject an honest response. Counters that Oxyn bounds itself keep
  `.int()`;
- a `Channel` message goes through `guarded`: same boundary, but an unreadable
  message is dropped with a trace, because nobody is waiting for it.

`Cell` is validated by a hand-written positional test, not by a `z.union`. It is
a **measured** decision — the comment carries the figures. Do not "simplify" it.

## Components

| Use | Never | Why |
|---|---|---|
| a component from `src/components/ui` (shadcn, Base UI) | a styled `div` imitating a button, a menu, a dialog | keyboard, focus and ARIA come from Base UI; rewriting them means getting them wrong |
| `pnpm exec shadcn add <component>` | writing a shadcn component by hand | the repository's `shadcn` skill describes the rest — `render` and not `asChild` on Base UI |
| semantic tokens (`bg-background`, `text-muted-foreground`, `text-env-production`) | raw Tailwind colors, manual `dark:` | the theme and AA contrasts are set in `src/styles.css`, once |
| `@hugeicons/react` | `lucide-react` | [UX-SPEC](../../docs/UX-SPEC.md#first-workspace-navigation) mandates Hugeicons |
| `TextInput`, `TextArea`, `InputGroupTextInput`, `InputGroupTextArea` from `components/oxyn/text-field` | `Input`, `Textarea`, `InputGroupInput`, `InputGroupTextarea` from `components/ui` — ESLint refuses them | macOS would replace `'` with `’` in a connection string ([ADR-0041](../../docs/adr/0041-registre-d-actions-menus-et-raccourcis.md) § 8); any other field (CodeMirror, Lexical, `CommandInput`) receives `TEXT_FIELD_ATTRIBUTES` |
| React text (`{value}`) | `dangerouslySetInnerHTML` on received data | a cell, an object name or a model response are hostile inputs ([SECURITY](../../docs/SECURITY.md#input-surface)) |

`src/components/ui` is **generated**: it is neither formatted nor linted by the
project, and a touch-up there is overwritten at the next
`shadcn add --overwrite`. What is specific to Oxyn goes in
`src/components/oxyn`.

**A single exception, dated 2026-09-25**: `src/components/ui/select.tsx` passes
the name of its trigger to the open list — its `aria-labelledby` if it has one,
otherwise its `aria-label` — and accepts an explicit `aria-label` on
`SelectContent`. Base UI leaves the listbox unnamed: axe reports
`aria-input-field-name`, and a screen reader announces an anonymous list. The
trigger is not targeted by `aria-labelledby`: a combobox referenced that way
lends its chosen value, not its label. The touch-up is commented at the top of
the file; the `ListIsNamedAfterItsTrigger` story of
`assistant-reasoning-effort.stories.tsx` fails if a regeneration erases it. It
goes away the day Base UI names the list itself.

## Every Oxyn component has its stories, and its stories are its tests

A component of `src/components/oxyn` that depends on a remote operation has one
story **per state**: initial, in progress, populated, empty, error
([UX-SPEC](../../docs/UX-SPEC.md#states-of-a-view)). `make front` renders them in
Chromium and runs axe on them in `error` mode: an accessibility violation fails
the gate.

`make front` refuses a component that no story renders
(`script/verifier-stories`). The rule it applies:

- **a component** is a `.tsx` file of `src/components/oxyn`, excluding
  `*.stories.tsx` and `*.test.tsx`. What renders nothing — model, fixtures,
  measurement, utility — is written as `.ts` and is not concerned: the
  extension is the boundary, and a `.tsx` without JSX has no reason to exist;
- **it is covered** if a `*.stories.tsx` in the same directory imports it
  (`./<name>` or `@/components/oxyn/<name>`): its own stories usually, or those
  of the view that renders it directly (`result-chart.stories.tsx` for
  `assistant-result-chart`);
- **otherwise, it is exempted by name** in the script's `EXEMPTS`, and only in
  two cases: a piece that only makes sense inside its parent (Lexical plugin,
  chart plot), whose exemption names the story that renders it; or a pending
  story, whose exemption carries a dated `TODO` saying what unblocks it. An
  exemption that is no longer used fails the check.

Behaviors that protect the user are written as `play`, not as comments: `Cancel`
focused in an approval, Enter that does not approve, a connection credential
never rendered.

A story never talks to the backend. The component receives its data and
callbacks through props; `src/features` makes the link with `src/lib/ipc`.

## The result is never whole in the webview

The grid requests bounded pages (`result_page`, at most 2,000 rows on the Rust
side) for the visible window only. Loading "the whole result to sort in JS"
reintroduces the OOM that [I-06](../../CLAUDE.md#i-06) forbids, with a webview
that dies instead of a process.

Cell formatting is done in Rust by `oxyn_data::format_cell`. The front end does
not reformat a date or a binary: the screen would diverge from the exported file.

## No retry

`QueryClient` is configured with `retry: false`. A failed backend call is an
answer, not a flaky network; and an ambiguous write is not replayed
([I-13](../../CLAUDE.md#i-13)). `retryable` comes from the backend, never from
parsing the message.

## CSP

The production CSP is in `crates/oxyn-desktop/tauri.conf.json` and stays strict.
`tauri.dev.json5` removes it **in development only**, because Tauri injects
nonces there that block Vite's inline scripts: the window stays blank, without
an error. A blank page in production is therefore never "the CSP to loosen": it
is a new inline script that needs to be understood.

## Versions

Exact in `package.json`, read from the registry and dated in
[RESEARCH-NOTES](../../docs/RESEARCH-NOTES.md#tauri-interface-and-front-end)
([I-12](../../CLAUDE.md#i-12)). `shadcn add` writes `^`s: remove them in the same
commit. Vitest stays on 4 as long as `@storybook/addon-vitest` does not accept 5.

## Verify

```bash
make front          # format, lint, types, unit tests and stories, build
make desktop-dev    # the real window, on a temporary workspace
```
