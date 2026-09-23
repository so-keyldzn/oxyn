---
name: base-ui-select-faux-positifs
description: Select base-nova — `SelectValue placeholder` est valide en Base UI 1.8 (le skill dit le contraire) ; un SelectItem hors SelectGroup est un défaut visuel, pas seulement sémantique
metadata:
  type: feedback
---

Deux lectures du skill à corriger sur le `Select` Base UI :

1. **`<SelectValue placeholder="…" />` n'est pas une violation.** `base-vs-radix.md`
   dit que Base exige un élément `{ value: null }` dans `items` ; le `.d.ts` de
   `@base-ui/react` 1.8 (`select/value/SelectValue.d.ts`) déclare bien une prop
   `placeholder?: React.ReactNode`. Ne pas « corriger » un placeholder existant.
2. **Oublier `SelectGroup` coûte du rendu.** Dans le `select.tsx` généré
   (base-nova), `SelectContent` n'a **aucun padding** et c'est `SelectGroup` qui
   porte `p-1` : des items posés directement dans le contenu touchent le bord du
   popup. Le risque de la correction est donc `visuel`, pas `mécanique`.
   `DropdownMenuGroup`, lui, ne porte aucune classe (le `p-1` est sur le
   contenu) : l'y ajouter est mécanique.

Et un piège voisin, sur `Marker` : `MarkerIcon` rend `aria-hidden="true"`. Un
`Spinner` (qui porte `role="status"` et `aria-label`) placé dedans disparaît de
l'arbre d'accessibilité ; c'est voulu quand le `Marker` porte déjà le statut en
texte, pas si le spinner est la seule annonce.

**Why:** constaté le 2026-09-23 en passe de conformité sur l'assistant ; le
placeholder aurait été réécrit pour rien, et l'écart de padding n'était pas vu.

**How to apply:** avant de signaler un `Select`, lire le `.d.ts` de la version
installée plutôt que la règle du skill ; classer un ajout de `SelectGroup` en
`visuel`.
