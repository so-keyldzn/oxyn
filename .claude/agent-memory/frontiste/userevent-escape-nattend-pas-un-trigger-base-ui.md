---
name: userevent-escape-nattend-pas-un-trigger-base-ui
description: Dans un play Storybook, userEvent.keyboard("{Escape}") focus sur un Button qui est TooltipTrigger Base UI ne déclenche aucun onKeyDown React, alors qu'une vraie touche marche
metadata:
  type: feedback
---

Un `play` qui fait `bouton.focus()` puis `userEvent.keyboard("{Escape}")` **ne
déclenche aucun `onKeyDown` React** quand ce bouton est un `TooltipTrigger`
Base UI (`<TooltipTrigger render={<Button …/>}>`). `toHaveFocus()` passe, aucun
tooltip n'est ouvert, et pourtant le gestionnaire d'un ancêtre n'est jamais
appelé. La même touche envoyée par le vrai clavier (Playwright
`keyboard.press("Escape")` sur la même story) fonctionne.

**Why:** deux heures perdues à croire à un bug de mon gestionnaire Escape sur le
workspace Oxyn ; il fallait instrumenter le composant avec un `console.log` et
comparer story-runner vs navigateur réel pour voir que l'événement ne partait
pas. Depuis un `<input>` ou un `role=tab`, `userEvent.keyboard` marche très bien.

**How to apply:** pour prouver un raccourci clavier global dans une story, poser
le focus sur un élément **ordinaire** (champ, onglet) plutôt que sur un
déclencheur Base UI enveloppé. Si la story doit vraiment partir d'un bouton à
tooltip, vérifier d'abord dans un navigateur réel avant de conclure que le code
est faux — et instrumenter le composant plutôt que le test.
