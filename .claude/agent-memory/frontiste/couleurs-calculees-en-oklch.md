---
name: couleurs-calculees-en-oklch
description: Le thème est en OKLCH depuis le 2026-09-23 ; getComputedStyle rend oklch(...), donc une story qui lit une couleur par regex rgb() échoue — passer par un canvas
metadata:
  type: feedback
---

Une story qui mesure un contraste ne lit **jamais** une couleur calculée par
une regex `rgba?\(` : la peindre sur un canvas 1×1 et relire `getImageData`,
comme `paint()` dans `theme-contrast.stories.tsx`.

**Why:** `styles.css` est écrit en OKLCH depuis la refonte du 2026-09-23, et
Chromium rend alors `getComputedStyle(el).borderTopColor` sous la forme
`oklch(0.545 0.01 60)`. `assistant-sample-approval.stories.tsx` analysait du
`rgb()` : ses deux stories de contraste ont échoué sur « Unreadable colour »,
alors que le contraste réel passait. Heureusement l'analyseur échouait fort ; un
repli silencieux aurait mesuré 1:1 ou n'importe quoi.

**How to apply:** toute nouvelle mesure de couleur dans une story passe par un
canvas avec la sentinelle `#010203` (une couleur illisible garde la sentinelle
et fait échouer la story). Et une teinte de statut se vérifie aussi **sur sa
propre teinte** : `ui/` écrit `bg-destructive/10` en clair, `/20` en sombre,
`/30` au survol — c'est là qu'axe a trouvé 4,4:1, pas sur les surfaces nues.
