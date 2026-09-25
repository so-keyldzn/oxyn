---
name: premiere-story-paie-le-demarrage-a-froid
description: Une story lente sous charge et rapide seule est souvent la première de son fichier — axe-core et les imports à la demande tombent dans son budget de 15 s
metadata:
  type: feedback
---

Sous addon-vitest, chaque fichier de stories tourne dans un cadre neuf : axe-core
(chargé par l'`afterEach` d'addon-a11y), le premier rendu et tout `import()` à la
demande (mermaid, shiki) se paient dans la **première** story du fichier, sur son
budget de 15 s (défaut de Vitest en mode navigateur). Machine chargée : 15 s
dépassées, alors que la même story prend 700 ms seule.

**Why:** constaté le 2026-09-25 ; mesurer par un `beforeEach`/`afterEach` qui
logge `performance.now()` a montré que le temps était dans le rendu et l'axe à
froid, pas dans le `play`.

**How to apply:** ne pas allonger un `timeout` ; déclarer le chargement par
`preloadBeforeStories` dans le fichier de stories, le `beforeAll` de
`.storybook/vitest.setup.ts` le paie hors budget. Reproduire en vidant
`node_modules/.cache/storybook/*/*/sb-vitest/deps` : c'est **là** que vit le
cache d'optimisation des tests — addon-vitest écrase le `cacheDir` déclaré dans
`vite.config.ts`. `axe-core` n'est pas résolvable depuis le code du projet
(dépendance d'addon-a11y seulement) : on le chauffe en lançant une story vide
par `composeStory(...).run()`.
