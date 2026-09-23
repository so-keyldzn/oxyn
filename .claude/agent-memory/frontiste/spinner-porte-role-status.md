---
name: spinner-porte-role-status
description: le Spinner de components/ui porte role="status" + aria-label="Loading" — chaque spinner visible est une région live de plus
metadata:
  type: feedback
---

`Spinner` (`src/components/ui/spinner.tsx`, généré) rend `role="status"` et
`aria-label="Loading"`. Un panneau qui veut **une seule** région live (le
panneau de l'assistant) en compte autant de plus qu'il y a de spinners visibles.
Et un `getAllByRole("status")` compte aussi ceux-là.

**Why:** découvert en ramenant le panneau de l'assistant à une seule région
`status`. Les spinners dans un `MarkerIcon` sont déjà masqués (`aria-hidden`
sur le parent), mais pas ceux d'un `Badge`.

**How to apply:** à côté d'un libellé qui dit déjà l'état, passer
`aria-hidden` au `Spinner`. Dans une story, viser la région par un
`data-slot` plutôt que de compter les `status`. Voir aussi
[[spinner-change-le-nom-du-bouton]].
