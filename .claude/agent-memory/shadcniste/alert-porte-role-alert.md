---
name: alert-porte-role-alert
description: Le composant Alert généré pose role="alert" en dur ; migrer un encadré statique vers Alert en fait une région live annoncée, sauf à surcharger role
metadata:
  type: feedback
---

`ui/alert.tsx` rend `<div role="alert" … {...props}>` : tout `Alert` est une
région live assertive. Le `role` est posé **avant** l'étalement des props, donc
`<Alert role="note">` le remplace.

**Why:** la règle « Callouts use Alert » pousse à convertir un encadré
d'avertissement maison (bordure pointillée, icône) en `Alert`. Si l'encadré est
statique et monté à l'ouverture d'un dialogue, la conversion le fait lire par
le lecteur d'écran avant le titre, et ajoute un `role="alert"` que les `play`
qui comptent les alertes (`getByRole("alert")` pour une erreur) voient en
double. Ni le typecheck ni axe ne signalent le changement.

**How to apply:** avant de migrer un encadré vers `Alert`, décider s'il doit
être annoncé. Statique → passer `role="note"` (ou le rapporter en `décision`) ;
erreur qui apparaît → garder le rôle par défaut. Vérifier les `play` qui
cherchent `alert` dans le même écran.
