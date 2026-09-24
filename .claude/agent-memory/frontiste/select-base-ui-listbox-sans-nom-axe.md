---
name: select-base-ui-listbox-sans-nom-axe
description: Une story qui ouvre un Select Base UI échoue à axe (aria-input-field-name) si la liste est encore là à la fin du play
metadata:
  type: feedback
---

Base UI 1.x ne nomme pas le `role="listbox"` de `Select` (ni `aria-label`, ni
`aria-labelledby`), et `SelectContent` ne transmet rien à `SelectPrimitive.List`.
axe tourne **après** le `play` : si la liste est encore montée (animation de
fermeture comprise), la story échoue en `aria-input-field-name`.

**Why:** rencontré sur les stories du graphique de l'assistant, 2026-09-24 ; les
stories d'`assistant-agent-settings` contournaient déjà ainsi.

**How to apply:** finir tout `play` qui ouvre un Select par
`await waitFor(() => expect(within(document.body).queryByRole("listbox")).toBeNull())`.
Ne pas retoucher `src/components/ui/select.tsx` (généré).
