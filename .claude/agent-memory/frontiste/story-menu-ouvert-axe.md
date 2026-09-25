---
name: story-menu-ouvert-axe
description: Une story qui finit avec un menu Base UI encore ouvert ou en fermeture échoue sur axe (aria-hidden-focus des focus guards)
metadata:
  type: feedback
---

Une story qui clique une entrée de menu contextuel (ou de sous-menu) et se termine
aussitôt échoue dans `make front` sur `aria-hidden-focus` : axe tourne après le
`play` et trouve les `span[data-base-ui-focus-guard]` du menu encore montés.

**Why:** le menu Base UI se démonte après son animation de fermeture ; axe passe avant.

**How to apply:** terminer chaque `play` qui ouvre un menu par
`await waitFor(() => expect(within(document.body).queryByRole("menu")).toBeNull())`,
y compris après un clic qui ferme le menu de lui-même. Entre deux ouvertures
successives, attendre aussi la fermeture, sinon `Escape` vise l'ancien popup.
