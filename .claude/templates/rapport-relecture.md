# Review report — <scope>

**Date:** AAAA-MM-JJ · **Scope:** `<paths or commit range>`

## Verdict

<One sentence. If there is nothing to report, this section is enough and the
report stops here. Do not invent remarks to justify the run: a report that always
finds something ends up no longer being read, and that is when the real problem
slips through.>

## Points, by decreasing severity

### 1. <short title> — blocking / to fix / to consider

| | |
|---|---|
| Where | `path/file.rs:42` |
| What is violated | I-NN, or the authoritative document and its section |
| Failure scenario | *What happens to a real user. Not the rule recited.* |
| Fix | … |

<Repeat per point.>

## Severity order

1. invariant violated — blocking
2. code / documentation divergence — blocking, it is a bug by definition
3. contract not honored
4. budget exceeded without a measurement justifying the gap
5. the rest

## What I am not sure about

<The doubts, named. A reported doubt is worth more than a fabricated certainty.>

## What was not reviewed

<The scope not covered, and why. A reader assumes everything was seen.>
