---
name: pieges-context-menu-et-stories
description: Pièges d'outillage Base UI ContextMenu (select-none, refus d'ouverture) et Storybook (vi absent, dialogue animé, pointer coords)
metadata:
  type: feedback
---

Pièges vus en écrivant les menus contextuels (lot 3, 2026-09-25).

- `ContextMenuTrigger` (shadcn sur Base UI) ajoute `select-none` au déclencheur : autour d'un champ éditable (CodeMirror), passer `className="select-text"` au trigger, sinon la sélection de texte hérite de `user-select: none`.
- Pour refuser l'ouverture sur une zone sans cible (bande d'onglets vide), rendre `ContextMenu` contrôlé et lire la cible dans un **ref** posé par l'`onContextMenu` de l'enfant : `onOpenChange` est appelé dans le même événement, avant que l'état React soit commité.
- `storybook/test` n'exporte pas `vi` : lire les appels par un `fn()` au niveau module (`mock.calls`).
- Un `Dialog` Base UI s'ouvre animé : `toBeVisible()` juste après `findByRole` échoue ; l'envelopper dans `waitFor`.
- `userEvent.pointer({ keys: "[MouseRight]", coords })` porte des coordonnées : les obtenir par un `Range` sur le nœud texte pour viser un mot dans CodeMirror (`posAtCoords`).

**Why:** chacun a coûté un aller-retour de tests ; aucun n'est visible à la lecture du code.
**How to apply:** à toute nouvelle surface de menu contextuel ou story qui ouvre un menu ou un dialogue.
