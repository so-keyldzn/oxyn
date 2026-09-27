# ADR-0031 — Every backend response is validated at the front end's entry

**Status:** accepted · **Date:** 2026-09-16

**Clarifies:** [ADR-0029](0029-interface-tauri-shadcn.md), which sets up the Tauri
interface and the IPC bridge without saying what the front end takes for
granted about what comes back through it.

## Context

`apps/desktop/src/lib/ipc/types.ts` has carried at its top, since its first
commit, the warning that describes the defect exactly:

> Mirror of `crates/oxyn-desktop/src/ipc.rs`. A field renamed on one side
> without the other fails at runtime only, so both change in the same commit.

The mirror is kept by hand, and nothing checks it. `call<T>()` calls
`invoke<T>()`, whose type parameter is a **cast**: TypeScript writes
`OpenConnection` over whatever the backend serialized, without looking. A field
renamed on one side only produces an `undefined` that propagates to an empty
render or a `TypeError` three screens away from its cause. The only check that
exists today is `isIpcError`, a hand-written guard for the error path, in
`client.ts`.

Two properties make this silence more expensive than elsewhere:

1. **What crosses is not only Rust we control.** An object name, a cell, a
   server error message and a model response come up through the same bridge.
   `docs/SECURITY.md` already classifies them as hostile inputs on the rendering
   side; at the entry, nobody looks at them.
2. **The divergence does not break the build.** Neither `tsc` nor the stories —
   which never talk to the backend — can catch it. It only shows up at runtime,
   in the user's window.

`zod@4.6.5` was already declared in `apps/desktop/package.json` and **imported
nowhere**: zero occurrences in the 289 files of `apps/desktop/src`. A dependency
with no use, and with no dated record in
[RESEARCH-NOTES](../RESEARCH-NOTES.md) ([I-12](../../CLAUDE.md#i-12)).

### What validation costs on the hot path

`read_result_page` is the most frequent call of the product: the grid emits it
on every scroll. What it requests is bounded by `pageSizeFor` in
`components/oxyn/result-grid.tsx` — `min(PAGE_SIZE 200, PAGE_CELLS 20 000 /
columns)`, i.e. **20,000 cells at worst**, never the 2,000 rows the command
accepts. The frame budget during a continuous interaction is **8 ms p99**
([PERFORMANCE](../PERFORMANCE.md#interaction-budgets)).

`Cell` is moreover the **only `untagged` type** that crosses the boundary:
validated as a union, each of its four shapes is tried in sequence. Measured on
2026-09-16 on the dev machine, `zod@4.6.5`, a full page of 20,000 cells:

| What validates `rows` | p50 | p99 |
|---|---|---|
| `z.union` of the four shapes | 2.26 ms | 7.72 ms |
| **positional discriminant** | **0.83 ms** | **1.16 ms** |
| envelope only, cells not read | 0.28 ms | 0.59 ms |

The union consumes almost the whole frame for the same guarantee as a test that
looks at the value instead of trying shapes. The discriminant costs 0.57 ms p99
more than checking nothing at all — and checks everything.

> **Caveat.** Measured under Node 22 (V8), not in the webview, which is
> WKWebView — JavaScriptCore — on macOS. The order of magnitude is enough to
> decide between the three rows; a figure taken under the real webview would
> move them together.
>
> **A first measurement, discarded**, was on pages of 2,000 rows — twice what
> the grid actually requests — and concluded 9.36 ms p99, hence a necessary
> exception on the row bodies. It is recorded here because the conclusion it
> called for was wrong, and it was almost kept.

## Decision

**Every backend response is validated by a `zod` schema at the front end's
entry. No exception, including on the grid's hot path.**

1. **`call` takes a schema, not a type parameter.** The signature becomes
   `call<T>(command: string, schema: z.ZodType<T>, args?)`, and the body parses
   the response before returning it. An `invoke` whose result is not parsed no
   longer exists. The single gateway of [I-01](../../CLAUDE.md#i-01) also
   becomes the single validation point.

2. **Types are derived from schemas.** `types.ts` and each
   `lib/ipc/<domain>.ts` declare a schema, and the type is
   `z.infer<typeof X>`. One declaration per type: a schema and a type can no
   longer diverge. The mirror with `ipc.rs` stays manual — it is the one that
   validation makes **loud** instead of silent.

3. **A validation failure is a non-retryable `BackendError`**, which names the
   faulty field and the command. It is not replayed:
   [I-13](../../CLAUDE.md#i-13) holds here as elsewhere, and a malformed
   response will still be malformed on the second call.

4. **`Cell` is validated by a positional discriminant, not by a union.**
   `isCell` looks at the value — `null`, string, then the presence of `text` or
   `unrenderable` — instead of trying four schemas one after the other. It is
   the only place where the validator's shape is dictated by a measurement, and
   the comment that goes with it carries the figures, so that nobody
   "simplifies" it into `z.union` without knowing what it costs.

5. **A `Channel` message is validated like a response.** `guarded` wraps the
   four channels — `ExecutionEvent`, `RefreshSignal`, `ShutdownSignal`,
   `AiUpdate`. An unreadable message is **dropped with a trace**, not thrown:
   an exception in `onmessage` would be swallowed by Tauri's internals.

6. **What governs a terminal state degrades instead of refusing.**
   Dropping in point 5 is justified by "nobody waits for this message". That is
   true of `ExecutionEvent` — the terminal state of a query comes from the
   `CommandOutcome` that `execute` resolves — and **false** of `AiUpdate`:
   `ai_ask` only resolves an `AskStarted`, and the only way out of the
   "in progress" state is a `finished` or `failed` event. A drop there leaves
   the turn spinning forever, and the thread then refuses any new question.

   So `Ending` and `FailureCategory` fall back to `unknown` rather than
   failing. `unknown` is a fallback **of the front end**, not a variant Rust
   emits, and it is classified as non-replayable: what could not be read is not
   replayed ([I-13](../../CLAUDE.md#i-13)).

   The general rule to retain: **before validating a field strictly, look at
   what rejecting it prevents**. Where the failure is silent and blocks a
   state, validation worsens the failure instead of naming it.

## Consequences

* **+** The divergence between `ipc.rs` and the front end becomes a named
  error, on the first call, instead of a propagated `undefined`. It is the
  defect that the comment at the top of `types.ts` announced without being able
  to prevent it.
* **+** The schema is executable: it says what the backend promises, and checks
  it. A `type` only asserts it.
* **+** An already installed dependency stops being dead weight, and gets its
  dated record.
* **−** The mirror stays manual, and it now has one more shape to keep:
  changing a field requires touching `ipc.rs` **and** the schema. Validation
  reports it at runtime; it does not avoid it.
* **−** A `Cell` validator whose shape is dictated by a measurement, hence
  fragile in review: it *looks like* a union written by hand out of ignorance of
  `z.union`. Only its comment defends it.
* **−** Three schemas escape the general rule and fall back instead of
  refusing — `Ending`, `FailureCategory`, and the two closed enums of
  `metadata.ts`. Each has its reason written next to it, but these are four
  places where a mirror divergence stays **silent**, that is, exactly what this
  ADR removes elsewhere. The compromise is accepted where rejection costs more
  than drift; it does not extend by analogy.
* **−** 1.16 ms p99 taken from the frame budget on every grid page, where there
  was nothing. That is 15% of the budget, measured under V8 and not under the
  real webview: the margin is comfortable, it is not infinite.
* **−** A non-zero parse cost on every other response. It is not measured
  individually; it is dominated by the IPC round trip.

**Exit cost:** going back means replacing `z.infer` with the `interface`s it
replaces and removing the `schema` argument from `call` — mechanical, and
bounded by the fact that everything goes through a single function. What does
not undo cheaply are the schemas themselves, if they have meanwhile
accumulated constraints (`.int()`, `.nonnegative()`) that describe the contract
better than `ipc.rs`.

**Reconsider if** one of these three things happens: a type generator from Rust
(`ts-rs`, `specta`) enters the repository — it would remove the manual mirror,
and this ADR would only have to cover validation; the parse of an ordinary
response shows up in a frame profile; or `pageSizeFor` stops bounding a page to
20,000 cells, in which case point 4 rests on a measurement that no longer
describes the worst case and must be redone.

## Rejected alternatives

| Alternative | Reason for rejection |
|---|---|
| Validate nothing, keep `invoke<T>` | This is the current state: a cast that nothing checks, whose failure mode is the silent `undefined`. The comment at the top of `types.ts` has described the defect from the start without being able to prevent it. |
| Validate `Cell` with a `z.union` of the four shapes | 7.72 ms p99 on a full page, against an 8 ms frame budget: the guarantee is the same as the discriminant's, the cost seven times higher. Since `Cell` is `untagged`, the union tries the shapes in sequence where a test tells them apart at a glance. |
| Validate the envelope and leave cells unread | The choice the first, wrong measurement called for. Once the real worst case was measured, it saves 0.57 ms p99 in exchange for the only hole in the boundary — on the path through which a third-party server's data arrives. |
| Keep the `interface`s and write the schemas next to them, matched by `z.ZodType<T>` | Smaller diff, but two declarations to keep per type. It is the manual mirror this ADR exists to reduce, duplicated once more inside the front end. |
| Generate the TS types from Rust (`ts-rs`, `specta`) | Removes the cause rather than the symptom, and remains the right answer in the long run. Rejected **here**: it is a dependency and a build step on the Rust side, hence a decision of its own, to be taken for itself and not as a by-product of introducing a validator. Named as a reconsideration condition. |
| Hand-written guards, like `isIpcError` | Holds for a type with two fields, not for the thirty or so the boundary carries. Each guard is untested code that asserts what it does not always check — and only one exists so far, for the error path, which shows the pace at which they get written. |
