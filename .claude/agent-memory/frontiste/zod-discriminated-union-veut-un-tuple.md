---
name: zod-discriminated-union-veut-un-tuple
description: zod 4 — building a z.discriminatedUnion from a .map() over .options fails the typecheck (TS2345); a tuple assertion fixes it
metadata:
  type: feedback
---

`z.discriminatedUnion("tag", variants)` requires a **non-empty tuple**
(`readonly [$ZodTypeDiscriminable, ...$ZodTypeDiscriminable[]]`). Passing the
result of an `Array.prototype.map` — the natural pattern to derive a union from
another, for example by adding a common field with `.extend()` — gives an
`Array<...>`, and `tsc` refuses:

```
error TS2345: … is not assignable to parameter of type
'readonly [$ZodTypeDiscriminable<string>, ...]'.
  Source provides no match for required element at position 0 in target.
```

**Why:** encountered on 2026-09-16 while converting `lib/ipc/` to zod schemas.
The error does not say "an assertion is missing", it talks about a missing
element at position 0, which points towards an empty array and wastes time.

**How to apply:** name the array, then assert it to its own element type.
Checked by the typecheck:

```ts
const variants = Kind.options.map((v) => v.extend({ command: z.string() }))
const Event = z.discriminatedUnion("type", variants as [
  (typeof variants)[number],
  ...(typeof variants)[number][],
])
```

Asserting to `$ZodTypeDiscriminable` directly **does not work** (TS2352,
"neither type sufficiently overlaps"): it is the element's type that is needed,
not the constraint's.
