---
name: cn-importe-du-paquet
description: Les fichiers générés par la CLI importent `cn` du paquet `cn`, pas de l'alias ; la bascule vers `@/lib/utils` est mécanique mais ne touche jamais ui/
metadata:
  type: feedback
---

`grep 'from "cn"'` sort ~80 fichiers, dont **tout `src/components/ui`** : la CLI
écrit l'import du paquet et non l'alias `utils` de `components.json`, et
l'exemple de `styling.md` fait de même. `@/lib/utils` ne fait que réexporter
`cn` : remplacer l'import dans un fichier Oxyn ne change rien à l'exécution.

**Why:** relevé le 2026-09-23 ; sans ce contexte, on prend le motif pour une
dérive d'un seul fichier, ou on est tenté de « corriger » aussi `ui/`.

**How to apply:** corriger en `mécanique` dans `src/components/oxyn` et
`src/features` (import placé après les autres `@/lib/…`, `import/order` est
désactivé) ; ne jamais toucher `ui/`, que le prochain `add` réécrirait. Voir
[[cn-supprime-les-tailles-de-theme]] (mémoire de `frontiste`).
