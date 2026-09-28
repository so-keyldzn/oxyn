---
name: pieges-de-la-porte-front
description: Three tooling traps that make prettier/eslint/vitest fail on new stories, and the command order that catches them
metadata:
  type: feedback
---

The order that catches everything without running `make front` (heavy, and it
includes the other agents' files):

```bash
pnpm exec prettier --check 'src/components/oxyn/<pattern>*'  # 1
pnpm -s typecheck                                            # 2
pnpm exec eslint <file> <file> …                             # 3
pnpm exec vitest run --project storybook <stories>           # 4
```

The three traps:

* **`prettier --check` then `eslint`, never the other way round.** Prettier's
  reformatting re-splits ternaries and objects, which moves the lines eslint had
  just flagged.
* **`--check` does not take several paths in a single shell variable**:
  `prettier --check $F` with `F="a b c"` returns "No files matching the pattern".
  Pass quoted patterns, one per argument.
* **`@typescript-eslint/no-unnecessary-condition` breaks story idioms.**
  `element.textContent ?? ""` is an error: in the repository's config,
  `textContent` of an `HTMLElement` returned by `getAllByRole` is typed non-null.
  Write `.map((item) => item.textContent)` plainly. Same rule on
  `TABLE[key] ?? default` when `TABLE` is a `Record<Union, V>`:
  `noUncheckedIndexedAccess` does **not** apply to a mapped type with literal
  keys. To keep a defensive read of a word coming from a protocol, go through
  `Object.hasOwn(table, word) ? table[word] : fallback` — it keeps compile-time
  exhaustiveness *and* the runtime fallback.
* **Two naming and configuration rules that surprise**:
  `@typescript-eslint/naming-convention` requires a type parameter named `T`
  or `T<Something>` (`<K extends string>` is refused); and
  `react/no-array-index-key` **is not configured** — an
  `// eslint-disable-next-line` that names it is itself an error
  ("Definition for rule … was not found").

And four test traps, not tooling ones but they cost the same round trip:

* `getByText` on a sentence a list repeats for two entries returns
  "Found multiple elements". Go through `getAllByRole("listitem").map(item =>
  item.textContent)` and compare the whole array;
* a sentence split by a `<strong>` or a `<span>` in the middle is **not** found
  by `getByText`: compare an ancestor's `textContent`, or
  `expect(element).toHaveTextContent(…)`;
* **a dialog is looked for in `document.body`, never in `canvasElement`**:
  it is portaled outside the story's root, and it **animates its entrance**.
  The repository's pattern (`approval-dialog.stories.tsx`) is
  `within(document.body)` + `findAllBy*` + `waitFor(() =>
  expect(x).toBeVisible())`. A lone `findBy*` resolves as soon as the node
  exists, i.e. while the dialog is still transparent, and the assertion fails
  with "Received element is not visible" — but **passes in isolation**, which
  sends you looking for an interference between stories that does not exist;
* **a disabled Base UI control does not have the `disabled` attribute.** A
  `Checkbox` renders `aria-disabled="true"` + `data-disabled` and stays
  focusable; `toBeDisabled()` fails on it. The `Button`, however, does use native
  `disabled` — so the rule is not uniform, you have to look at the component;
* **axe runs after the `play`, and sees a Base UI popup still closing.**
  A `Select` or a menu that animates leaves its focus guards
  (`[data-base-ui-focus-guard]`, `aria-hidden` and focusable): an
  `aria-hidden-focus` violation, and contamination of the next story. End every
  `play` that opens a popup with `await waitFor(() =>
  expect(document.querySelector("[data-base-ui-focus-guard]")).toBeNull())`
  — pattern of `preview-controls.stories.tsx`.

And two component traps the stories reveal:

* **Wrapping a control in a `TooltipTrigger` only when it is disabled remounts
  it** on state change: keyboard focus falls back to the page. Keep the wrapper
  permanently and toggle `<Tooltip disabled>`.
* **`InputGroup` carries `has-disabled:opacity-50`**: a single `disabled` child
  dims the whole group, field included, below readable contrast. A control that
  gets disabled (during an answer, for example) goes **outside** the group.

**Why:** each trap costs a full `vitest --project storybook` cycle (~10 s of
Chromium setup) for a trivial mistake.

**How to apply:** for every new component of `src/components/oxyn`. See also
[[axe-region-defilante-sans-focus]], which only shows up at step 4.
