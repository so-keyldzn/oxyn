---
name: alertdialogaction-est-un-button
description: Dans base-nova, AlertDialogAction est un simple Button avec data-slot, pas un Close Base UI ; remplacer un Button par lui ne ferme pas le dialogue et reste mécanique
metadata:
  type: feedback
---

Dans `alert-dialog.tsx` généré en base-nova, `AlertDialogAction` rend
`<Button data-slot="alert-dialog-action" …>` : aucune primitive
`AlertDialog.Close` derrière, contrairement à `AlertDialogCancel`. Un
`<Button variant="destructive">` posé dans `AlertDialogFooter` se remplace
donc par `<AlertDialogAction variant="destructive">` sans rien changer : même
élément, mêmes variantes, et la fermeture reste celle que le `onClick` pilote.

**Why:** constaté le 2026-09-23 ; on peut hésiter à faire la bascule par crainte
qu'elle ferme le dialogue avant que le rappel ait tourné (comportement Radix).

**How to apply:** relire `AlertDialogAction` dans `ui/alert-dialog.tsx` (une
mise à jour du registre peut changer la chose), puis classer la bascule en
`mécanique`.

Piège d'outillage voisin : `pnpm exec eslint` lancé depuis un sous-répertoire
d'`apps/desktop` échoue en « couldn't find any tsconfig.json » sur chaque
fichier ; le lancer depuis `apps/desktop`. Et sous zsh, une variable qui liste
plusieurs chemins n'est pas découpée : les passer un par un.
