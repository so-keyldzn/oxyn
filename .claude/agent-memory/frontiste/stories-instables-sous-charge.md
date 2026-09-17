---
name: stories-instables-sous-charge
description: "Sous charge (plusieurs agents), les stories qui lisent une frame d'animation Base UI échouent par intermittence : envelopper dans waitFor, et relancer avant de conclure à une régression"
metadata:
  type: feedback
---

Une story qui lit l'état d'un dialogue ou d'un popup Base UI **juste après son
ouverture** échoue par intermittence quand la machine est chargée. Envelopper
l'assertion dans `waitFor`, et relancer le fichier seul avant de conclure à une
régression.

**Why:** les surfaces Base UI entrent en transition d'opacité. `toBeVisible()`
lue sur la frame où l'élément est encore transparent échoue. Sous charge — la
CI locale, ou plusieurs agents qui lancent chacun Vitest/Playwright — la frame
lue change, donc l'échec apparaît en lot et disparaît en exécution isolée.
Observé aussi : `Failed to fetch dynamically imported module` et
`Port 63315 is already in use` quand deux exécutions se chevauchent, et une
violation axe `aria-activedescendant` pointant un id d'un popup en cours de
montage. Aucun de ces trois-là n'est un défaut du composant.

**How to apply:** avant de « corriger » un composant pour un échec de story,
relancer `pnpm exec vitest run --project storybook <le fichier>` seul. S'il
passe, corriger la **story** (ajouter `waitFor`) et non le composant. Ajouter
`NO_COLOR=1` pour que les motifs de `grep` retrouvent les lignes `FAIL` dans la
sortie.
