---
name: tabslist-line-souligne-coupe
description: Un TabsList shadcn variant="line" avec overflow-x-auto perd son soulignement d'onglet actif, et un h-* passé en className ne gagne pas contre le défaut du composant
metadata:
  type: feedback
---

Deux pièges du `TabsList` généré (`components/ui/tabs.tsx`, Base UI), qui se
manifestent ensemble et ne cassent aucun test :

**1. `variant="line"` dessine le repère d'onglet actif en `after:bottom-[-5px]`,
donc *hors* de la boîte du `TabsList`.** Ajouter `overflow-x-auto` pour faire
défiler une barre d'onglets rend aussi l'axe vertical scrollable (CSS : si un
axe n'est pas `visible`, `visible` devient `auto`) et **coupe le soulignement**.
L'onglet actif n'a alors plus aucun repère visible — ni fond ni bordure, le
variant `line` les mettant à `transparent`. Il faut donner au `TabsList` une
hauteur qui laisse les ~8 px sous le trigger (p. ex. remplir la barre de 48 px
plutôt que rester à 32 px).

**2. Un `h-8` sur le trigger dépasse la boîte de contenu d'un `TabsList` en
`h-8 p-[3px]`** (26 px utiles) : combiné au point 1, la rangée peut glisser
verticalement sous le pointeur. Vérifier `list.scrollHeight <= list.clientHeight`.

**Why:** trouvé sur la barre d'onglets du workspace Oxyn ; l'onglet actif n'était
distinguable que par la graisse du texte, et personne ne l'avait vu parce
qu'aucune story ne regarde la géométrie du `::after`.

**How to apply:** dès qu'on met `overflow-x-auto` sur un `TabsList variant="line"`,
écrire une story qui assert `trigger.bottom + 5 + 2 < list.bottom` **et**
`list.scrollHeight <= list.clientHeight`.

**Le corollaire tailwind-merge :** le défaut vient de la cva sous le variant
`group-data-horizontal/tabs:h-8`. Un `h-12` nu dans `className` ne le remplace
pas — tailwind-merge les traite comme deux clés différentes et la spécificité
CSS (`.group[data-orientation=horizontal] .h-8`, 0-2-1) l'emporte sur `.h-12`
(0-1-0). L'override doit **porter le même variant** : `group-data-horizontal/tabs:h-12`.
Vaut pour tout utilitaire que le composant généré pose déjà sous un variant.

Voir [[userevent-escape-nattend-pas-un-trigger-base-ui]].
