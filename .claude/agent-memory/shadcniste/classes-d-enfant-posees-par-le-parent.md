---
name: classes-d-enfant-posees-par-le-parent
description: Les composants base-nova stylent leurs enfants depuis le parent (`*:data-[slot=…]:`, `[&_svg]:size-4`) — un className sur l'enfant peut être redondant ou perdant, et cn ne le voit pas
metadata:
  type: feedback
---

Plusieurs composants générés posent la classe de l'enfant **sur le parent** :
`Alert` `destructive` porte `*:data-[slot=alert-description]:text-destructive/90`,
`SidebarMenuButton` porte `[&_svg]:size-4` (descendant, pas seulement `>svg`).
`cn` ne fusionne que dans une même liste : il ne voit jamais le conflit entre
le parent et l'enfant.

Conséquences en relecture :

- une icône imbriquée à n'importe quelle profondeur dans un `SidebarMenuButton`
  (la tuile de logo en `div`) a déjà `size-4` : le `className="size-4"` est
  redondant, sa suppression est `mécanique` ;
- un `text-foreground` passé à un `AlertDescription` sous `variant="destructive"`
  entre en concurrence avec la règle du parent, de spécificité plus forte : il
  est probablement sans effet. Ne pas le retirer comme « mécanique » sans avoir
  mesuré le rendu ; c'est un doute à signaler.

**Why:** constaté le 2026-09-23 en passe de conformité sur `src/features` ; la
lecture du seul `className` de l'enfant fait conclure à tort à une surcharge
qui gagne, ou à un `size-*` nécessaire.

**How to apply:** avant de classer un `size-*` ou une couleur sur un enfant de
composant, grepper le composant parent dans `ui/` pour `[&_`, `[&>` et `*:`.
