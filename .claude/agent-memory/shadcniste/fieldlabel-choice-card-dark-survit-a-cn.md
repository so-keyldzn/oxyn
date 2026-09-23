---
name: fieldlabel-choice-card-dark-survit-a-cn
description: Recolorer une carte de choix FieldLabel par className ne suffit pas — ses classes dark:has-data-checked:* ne sont pas fusionnées par cn et gagnent en sombre
metadata:
  type: feedback
---

Le motif « choice card » de shadcn (`FieldLabel` > `Field` + `RadioGroupItem`)
porte dans `field.tsx` deux jeux de classes pour l'état coché :
`has-data-checked:border-primary/30 has-data-checked:bg-primary/5` **et**
`dark:has-data-checked:border-primary/20 dark:has-data-checked:bg-primary/10`.

Passer `className="has-data-checked:border-<jeton>"` remplace le premier jeu
(même groupe, mêmes modificateurs pour tailwind-merge) mais **pas** le second :
`dark:has-data-checked:` est une autre liste de modificateurs, donc aucun
conflit détecté. En thème sombre, la carte redevient teintée `primary`.

**Why:** migrer une carte colorée par jeton vers ce motif oblige soit à écrire
un `dark:` manuel (interdit par styling.md), soit à toucher `ui/field.tsx`
(interdit). Ni le typecheck ni axe ne le voient.

**How to apply:** une carte de choix dont la couleur cochée dépend d'une valeur
(environnement, statut) reste en `label` maison ; la migration est une
`décision` à rapporter, pas une correction `visuel`. Vaut pour tout composant
de `ui/` qui double ses classes d'état sous `dark:`.
