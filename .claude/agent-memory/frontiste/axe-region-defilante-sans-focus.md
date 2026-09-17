---
name: axe-region-defilante-sans-focus
description: Un conteneur qui défile sans contrôle focalisable dedans fait échouer axe en mode error (scrollable-region-focusable) — ajouter tabIndex={0}
metadata:
  type: feedback
---

Tout élément avec `overflow-y-auto` / `overflow-x-auto` qui ne contient **aucun
élément focalisable** échoue la règle axe `scrollable-region-focusable`, et
comme `a11y.test` vaut `error` dans `.storybook/preview.ts`, c'est la porte
`make front` qui tombe. Le correctif est `tabIndex={0}` sur le conteneur, plus
`outline-none focus-visible:ring-2 focus-visible:ring-ring/60` pour que le focus
se voie.

**Why:** un `<ol>` ou un `<ul>` borné par `max-h-*` n'est atteignable qu'à la
souris sans cela ; un utilisateur au clavier ne peut pas lire ce qui dépasse.
Rencontré sur une liste d'étapes de 40 éléments, puis sur une liste de sources.

**How to apply:** dès qu'on borne une hauteur ou une largeur avec `overflow-*`
dans `src/components/oxyn`. Un conteneur qui contient déjà des boutons — par
exemple `AttachmentGroup` — satisfait la règle tout seul et n'a pas besoin de
`tabIndex`. Ne pas confondre : c'est le **contenu focalisable** qui exempte, pas
le rôle ni le `aria-label`.
