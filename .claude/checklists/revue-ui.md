# Interface review

Behaviors in `docs/UX-SPEC.md`, budgets in `docs/PERFORMANCE.md`.

## Execution path

- [ ] The view produces `Command`s; it calls no driver, not even "for the time
      being"
- [ ] No `block_on`, `blocking_*`, nor I/O on the UI thread

## Tauri interface

Conventions in [front.md](../rules/front.md), move in
[`/ecran`](../commands/ecran.md).

- [ ] `invoke` only appears in `src/lib/ipc/client.ts`
- [ ] Every new Tauri command is `async` or only reads in-memory state
- [ ] The TypeScript mirror changed in the same commit as `src/ipc`
- [ ] One story per state, and `make front` passes them, axe included
- [ ] An added Tauri command was reviewed by `relecteur-securite`

## The five states

- [ ] Initial — explains what to do
- [ ] In progress — progress **and a way to cancel**, never a mere freeze
- [ ] Populated
- [ ] **Empty** — visibly distinct from an error. It is the one people forget,
      and the first one a new user sees
- [ ] Error — what failed, whether it is retryable, the next action

## Cancellation

- [ ] Every operation exceeding 300 ms is cancellable
- [ ] Cancellation reaches the **server** — otherwise the button lies and leaves
      a connection taken

## Capabilities

- [ ] No surface assumes tables, a schema or SQL without checking the session's
      capabilities
- [ ] What is unavailable is **explained**, not hidden without reason nor failed
      without a message

## Writes

- [ ] No optimistic display on an operation that writes
- [ ] On `production`: confirmation naming the connection, exact SQL, estimate
      of affected rows
- [ ] The default button is never the destructive action

## Grid and results

- [ ] Grid: bounded pages from `result_page`, for the visible window only, never
      the whole result
- [ ] Scrolling beyond the memory budget reads a disk page and **never re-runs
      the query**
- [ ] No decoding of a large batch on the UI thread

## Displayed errors

- [ ] The server message is shown, code included — not a paraphrase
- [ ] No connection credential, no bound value, including in a debug panel

## Accessibility — blocking

- [ ] Every new view is keyboard-reachable
- [ ] Focus is visible
- [ ] Tab order follows reading order
- [ ] No information carried by color alone

It is not a finishing touch: Base UI provides the keyboard, focus and ARIA of a
component, but not those of a grid, a tree or an editor written for Oxyn.
Catching up afterwards costs a rewrite.

## Performance

- [ ] Budgets measured with the system's instruments, **not** with `criterion`
- [ ] A budget that cannot be held is amended **by an ADR**, never silently
