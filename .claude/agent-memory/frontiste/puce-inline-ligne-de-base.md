---
name: puce-inline-ligne-de-base
description: Une puce inline-flex dont le premier enfant est une icône SVG prend le bas de l'icône pour ligne de base et flotte au-dessus du texte ; marquer seul le texte `self-baseline`
metadata:
  type: feedback
---

Une puce `inline-flex items-center` posée dans une ligne de texte, avec une
icône en premier enfant, **n'est pas alignée** : la ligne de base d'un conteneur
flex est celle du premier enfant, et un SVG n'en a pas, donc c'est son bord bas
qui sert. La puce monte de 1 à 1,5 px, grossit la ligne, et le caret saute.

Ce qui tient : la puce à la taille du texte (pas de `text-xs`), `align-baseline`,
le nom seul en `self-baseline` (il devient la ligne de base du conteneur),
l'icône en `self-center` à `size-[1em]`, `leading-[1.25]` et une bordure
transparente sur toutes les variantes pour que la variante pointillée garde la
même boîte.

**Why:** retour de l'utilisateur sur la vraie fenêtre ; `toBeVisible` ne voyait
rien. Une mesure par boîtes (centre de la puce contre le rect `Range` du texte
voisin, et hauteur du bloc contre un clone où chaque puce devient un
inline-block de hauteur 0) échoue avec l'ancien style à 1,06 px et 1,3 px.

**How to apply:** à toute pastille inline dans du texte (mentions, badges dans
une bulle). Vérifier qu'une story de mesure échoue sans le correctif avant de
s'y fier. Voir [[lexical-dans-les-stories]].
