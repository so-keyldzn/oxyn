---
paths:
  - "**/*.rs"
---

# Rust — conventions

The fundamental prohibitions are in [CLAUDE.md](../../CLAUDE.md#invariants).
This rule carries what only holds for Rust code.

## Errors

| Where | What | Why |
|---|---|---|
| Library crate | `thiserror`, one enum per boundary | the caller must be able to tell cases apart without reading a string |
| `oxyn-desktop`, tests, benches | `anyhow` | nobody matches on variants at the top of the stack |
| Never | `Box<dyn Error>` in a public API | erases the information at the very moment it is needed |

A driver error carries its **class** — transient, permanent, ambiguous
([DRIVER-CONTRACT](../../docs/DRIVER-CONTRACT.md#4-it-distinguishes-three-families-of-errors-and-classifies-them)).
The class is data, not a deduction the caller makes from the message: a message
changes, and a caller that parsed it breaks silently.

## Constructs to use / never

| Use | Never | Why |
|---|---|---|
| `?`, `let … else`, `match` | `unwrap()`, `expect()` outside tests | [I-09](../../CLAUDE.md#i-09) |
| `u32::try_from(n)?` | `n as u32` | `as` truncates silently; an `id` of 5 billion becomes 705,032,704 |
| `slice.get(i)` | `slice[i]` on an index that comes from data | panics on server input |
| `#[non_exhaustive]` on public enums | closed public enum | adding a variant becomes a major break |
| `impl Trait` as argument | needless generic | less monomorphization, readable signatures |
| `Cow<'_, str>` when copying is rare | `String` everywhere | but do not contaminate five signatures for one call per window opening |

`expect()` is tolerated in a test, a bench, or an initialization whose failure
is a programming bug — never on a path reachable from the network. When it is
used, its message states **the assumed invariant**, not "failed".

## Async

- a public `async` function never blocks: every blocking call goes to the
  blocking pool ([ARCHITECTURE](../../docs/ARCHITECTURE.md#le-modèle-de-threads));
- every operation that can take time accepts cancellation **and propagates it to
  the server** — a dropped future does not release a remote query
  ([I-13](../../CLAUDE.md#i-13) and the driver contract);
- no detached `tokio::spawn` whose handle nobody holds: a task that can be
  neither cancelled nor awaited outlives the closing of the tab;
- beware of cancellation in the middle of a `select!`: the dropped future may be
  dropped **after** consuming bytes from the stream, leaving the decoder out of
  sync. A resumption point is designed, not improvised.

## Allocations

No mechanical rule. A single one that holds:

**Clear code is not replaced by fast code without the measurement showing it
was worth it** ([PERFORMANCE](../../docs/PERFORMANCE.md#the-rule-that-prevents-gratuitous-optimization)).

On the other hand, on a **per-row or per-value** path — conversion to
`RecordBatch` is one —, one allocation per element is a design defect from the
moment it is written, not an optimization for later.

## Public APIs

- every public item carries a `///` that says what is not in the signature:
  preconditions, what panics, what allocates, what blocks;
- `#[must_use]` on anything whose ignored result is a bug;
- no public trait with a single implementation that is not a boundary
  ([CLAUDE.md](../../CLAUDE.md#code-organization));
- a trait meant to cross the WASM boundary respects the constraints of
  [PLUGIN-CONTRACT](../../docs/PLUGIN-CONTRACT.md#what-this-contract-imposes-on-todays-traits)
  **starting today**: fixing them in phase 4 will cost a redesign.

## `unsafe`

Policy in [SECURITY](../../docs/SECURITY.md#unsafe-policy). The point that
gets missed: a `// SAFETY:` that paraphrases the code is worthless. It says
**why** the condition is true here and **who** will keep it true.

## Tests

The conventions live in [tests.md](tests.md): what gets tested, hostile inputs,
the levels of interface tests, benches.

The pointer is still needed, but not for the reason that used to be given here:
the `paths:` of [tests.md](tests.md) now covers `**/*_tests.rs` and
`**/tests.rs` in addition to directories. What it still does not cover is a
`#[cfg(test)] mod tests` written **at the bottom of a source file** — and that
is the prevailing form here. The conclusion held; the example that justified it
had aged.

## Verify

```bash
make qualite
```
