---
name: axe-ne-teste-pas-le-contraste-non-textuel
description: Stories vertes ≠ contraste vérifié — axe ignore WCAG 1.4.11 et Storybook rend en sombre par défaut ; mesurer par canvas, jamais par regex sur rgba()
metadata:
  type: feedback
---

Une story qui passe axe en mode `error` ne prouve **que** le contraste du texte,
et **que dans le thème rendu**. Deux angles morts :

1. **Le thème par défaut de Storybook ici est sombre** (`initialGlobals: {
   theme: "dark" }` dans `.storybook/preview.tsx`). Sans story portant
   `globals: { theme: "light" }`, le clair n'est jamais vérifié.
2. **axe ne teste pas le contraste non textuel** (WCAG 1.4.11) : bordure d'une
   case à cocher, contour d'un champ, piste d'un interrupteur, anneau de focus.

**La garde existe désormais pour les jetons de thème** :
`src/components/oxyn/theme-contrast.stories.tsx` mesure `--input` et `--ring`
dans les deux thèmes (corrigés le 2026-09-16 après des mesures à 1,45:1). Un
défaut de jeton s'écrit **là**, une fois, pas dans la story d'un écran. Elle ne
couvre pas tout : l'interrupteur, par exemple, n'y est pas (piste à 2,96:1 en
sombre, rattrapée par un curseur à 15,51:1).

**Comment mesurer — la leçon qui coûte cher :** réutiliser la méthode `paint`
de cette story (peindre la couleur sur un canvas 1×1 au-dessus du fond, relire
le pixel). Elle lit nativement `oklab`, `color-mix` et l'alpha, et échoue
bruyamment sur une couleur illisible. Un analyseur maison par regex s'est trompé
deux fois : `rgba(0, 0, 0, 0)` lu comme **noir opaque** (faux succès sur fond
clair, faux échec sur fond sombre), et `oklab(… / 0.8)` illisible. Et ne pas
déduire une couleur de `styles.css` sans la mesurer : `--input` avait changé
sous mes yeux.

Mesurer aussi **ce qui identifie réellement le contrôle** : pour un interrupteur
décoché, la bordure, la piste **et** le curseur — sinon on signale un faux
défaut.

**Why:** la question posée était « relis en sombre » ; les vraies lacunes étaient
le clair et le non-textuel, et ma première mesure était fausse parce qu'elle
ignorait l'alpha.

**How to apply:** avant d'affirmer un contraste, le mesurer dans le navigateur
par la méthode canvas, dans chaque thème, et éprouver le test par mutation. Voir
[[axe-region-defilante-sans-focus]], [[pieges-de-la-porte-front]].
