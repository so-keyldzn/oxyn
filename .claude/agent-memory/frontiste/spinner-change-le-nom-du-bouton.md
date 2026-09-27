---
name: spinner-change-le-nom-du-bouton
description: A Spinner in a Button adds "Loading" to the accessible name: getByRole({ name: "Declare" }) fails in the in-progress state
metadata:
  type: feedback
---

The `Spinner` of `components/ui` carries `role="status"` and
`aria-label="Loading"`. Placed in a `Button` (`<Spinner data-icon="inline-start" />`),
it enters the accessible name: the button is called "Loading Declare". An
"in progress" state story that looks for `getByRole("button", { name: "Declare" })`
returns `TestingLibraryElementError`, without saying why.

**Why:** lost a story round trip on `provider-form.stories.tsx` (2026-09-23).

**How to apply:** in a story where the button may carry a spinner, search with
`name: /Declare$/`. See also [[pieges-de-la-porte-front]].
