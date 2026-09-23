---
name: port-vitest-browser-partage
description: « Port 63315 is already in use » avec « no tests » = un autre agent lance ses stories en même temps, pas une panne
metadata:
  type: feedback
---

`vitest run --project storybook` écoute sur un port fixe (63315). Quand un
autre agent de l'équipe lance ses stories au même moment, le run finit en
`Error: Port 63315 is already in use` puis `Test Files no tests`. Ce n'est ni
un échec ni une régression.

**Why:** constaté pendant un travail à plusieurs agents sur `apps/desktop`.

**How to apply:** relancer dans une boucle qui détecte « already in use » et
attend ~20 s entre deux essais (le `sleep` de premier plan est bloqué :
`perl -e 'select(undef,undef,undef,20)'` passe).
