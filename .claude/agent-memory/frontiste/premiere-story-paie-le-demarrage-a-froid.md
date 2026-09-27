---
name: premiere-story-paie-le-demarrage-a-froid
description: A story that is slow under load and fast alone is often the first of its file — axe-core and on-demand imports fall into its 15 s budget
metadata:
  type: feedback
---

Under addon-vitest, each story file runs in a fresh frame: axe-core (loaded by
addon-a11y's `afterEach`), the first render and every on-demand `import()`
(mermaid, shiki) are paid in the **first** story of the file, on its 15 s budget
(Vitest's default in browser mode). On a loaded machine: 15 s exceeded, while
the same story takes 700 ms alone.

**Why:** observed on 2026-09-25; measuring with a `beforeEach`/`afterEach` that
logs `performance.now()` showed the time was in the cold render and axe, not in
the `play`.

**How to apply:** do not lengthen a `timeout`; declare the loading with
`preloadBeforeStories` in the story file, the `beforeAll` of
`.storybook/vitest.setup.ts` pays it outside the budget. Reproduce by emptying
`node_modules/.cache/storybook/*/*/sb-vitest/deps`: **that** is where the tests'
optimization cache lives — addon-vitest overrides the `cacheDir` declared in
`vite.config.ts`. `axe-core` cannot be resolved from the project's code
(dependency of addon-a11y only): it is warmed up by running an empty story
through `composeStory(...).run()`.
