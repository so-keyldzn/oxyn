# ADR-0024 — The draft is written when typing stops, not on every keystroke

**Status:** proposed · **Date:** 2026-09-11

**Clarifies:** [ADR-0016](0016-autosauvegarde-bornee.md), on the moment a
draft leaves the interface thread.

## Context

A measurement on 2026-09-11 found the only budget overrun of the whole
product. `xcrun xctrace`, `Animation Hitches` template, attached to the process:

| Condition | Hitches in ~14 s | Worst |
|---|---|---|
| Idle | 0 | — |
| Continuous resizing | 1 | 10 ms |
| **Continuous typing** | **17** | **50 ms** |

The budget of [PERFORMANCE](../PERFORMANCE.md#interaction-budgets) is **8 ms
p99** during a continuous interaction, and typing is explicitly one of the
three interactions targeted. Fifty milliseconds is six times the budget, on
the most frequent action of the product.

The cause is in the code, not in rendering. On every `EditorEvent::Changed`,
`QueryConsole::document_changed`:

1. calls `editor.text()`, which is a `self.lines.join("\n")` — hence a
   **full copy of the document**;
2. builds a whole `QueryDocumentUpdate`, text included;
3. validates it;
4. enqueues it in the write queue.

The first three steps are on the interface thread. `MAX_QUERY_DOCUMENT_BYTES`
is 1 MiB: at the bound, **every typed character copies a megabyte** before the
queue has anything to do.

What is **not** at fault, and had to be ruled out: the editor's rendering is
virtualized by `uniform_list` and only draws the visible lines;
`oxyn_query::current_statement` takes 55 µs and is only called on submit.
ADR-0016 did bound the **queue** — one active operation, at most one pending
draft — but said nothing about the cost paid **before** entering it.

## Decision

**A keystroke marks the draft dirty; it does not copy it.** The copy, the
validation and the send only happen after **250 ms without typing**, or
immediately when the console loses focus, closes, or an execution is
launched.

The delay restarts on every key: continuous typing therefore writes nothing
while it lasts, and writes once when it stops. The interface thread only
carries a flag and a timer.

**250 ms, and why this number.** It is below the 300 ms budget of "visible
feedback after a keystroke": a user who stops typing cannot perceive the delay
of the write, since it is shorter than the delay beyond which they would stop
connecting it to their action. It is also longer than the interval between two
keys of fast typing — around 100 ms —, which is the condition for a sentence
typed in one go to produce **only one** write. It is not a measurement: it is a
product choice, and it is amended here.

**What the 250 ms window costs, stated plainly.** An abrupt stop within that
interval loses up to 250 ms of typing — a few characters. Before this decision,
it lost none. It is the price, and it is paid knowingly: an editor that
stutters on every key on a long document is a flaw the user suffers every
second, whereas losing a few characters requires a crash.

**The three escapes are immediate**, and they are what bound the loss: loss of
focus, console closing, launch of an execution. The first two cover the
ordinary action — one leaves a tab, goes elsewhere; the third guarantees that
what executes is what is written.

**What does not change**: ADR-0016's queue, its expected revisions, its closing
counter, and the fact that a draft suspended by a cancelled closing resumes.
This decision only touches the moment one enters the queue.

## Consequences

- **+** The interface thread no longer copies the document on every key: it is
  the measured cause of the product's only budget overrun.
- **+** A sentence typed in one go produces one SQLite write instead of one per
  character. On a laptop, it is also one less disk write per keystroke.
- **+** The cost stops depending on the document's **size**. Today, the longer
  a draft is, the more each key costs — the degradation is therefore invisible
  on a short document and severe on the one being worked on for an hour.
- **−** Up to 250 ms of typing lost on an abrupt stop. The recovery screen of
  [ADR-0021](0021-marqueur-d-arret.md) then restores a text within a few
  characters of what was on screen.
- **−** One more timer in the console, hence one more state to undo correctly
  on closing — and a test to write for it, without which a write arrives after
  the closing it was supposed to precede.
- **−** The delay is a setting that is not one: it is exposed nowhere, and
  changing it requires coming back here.

**Exit cost:** remove the timer and call the copy directly again in
`document_changed`. The queue, the revisions and recovery do not depend on this
decision.

**Reconsider if** a measurement shows the copy is no longer the dominant cost —
for example if `TextBuffer` stops being a `Vec<String>` joined on demand —,
or if usage shows that 250 ms lost is too much, in which case the answer is a
shorter delay, not removing the timer.

## Rejected alternatives

| Alternative | Reason for rejection |
|---|---|
| Let the queue read the text at write time | The queue runs on the Tokio runtime and cannot read a GPUI entity; it would need a reverse channel to the interface thread, hence the same cost moved |
| Copy only the modified lines | Requires an incremental draft format, hence a migration and one more format to carry — for a gain a debounce obtains without changing storage |
| Write on every key but off the interface thread | The copy itself is the cost, and it can only happen where the editor lives |
| A debounce by number of characters rather than by time | A ten-second pause after three characters would write nothing: the criterion that matters is stopping, not volume |
| Lengthen the delay to 1 s to write even less | Loses a second of typing on a crash, for zero gain: typing is already coalesced at 250 ms |
