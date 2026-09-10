---
name: gpui-entite-enfant-et-focus
description: Piège GPUI — remplacer une Entity enfant au lieu de la muter détruit son FocusHandle et son abonnement ; préférer une méthode de mise à jour sur le composant
metadata:
  type: feedback
---

Sous GPUI, le focus vit dans le `FocusHandle` que l'entité a créé. Remplacer
l'entité (`self.field = cx.new(…)`) au lieu de la muter **perd donc le focus
clavier**, sans erreur ni avertissement — et perd aussi l'abonnement, qu'il faut
recréer, avec le risque d'en avoir deux vivants.

**Why:** un composant `SelectField` qui ne prenait ses options qu'à la
construction m'a poussé à reconstruire l'entité quand le catalogue arrivait. Un
utilisateur posé sur le contrôle au moment où la liste se remplissait se
retrouvait sans focus. Le défaut ne se voit à la compilation, ni aux tests tant
qu'aucun test n'assère le focus.

**How to apply:** quand un composant partagé ne peut être mis à jour qu'en le
reconstruisant, c'est le **composant** qu'il faut compléter (une méthode
`set_…(…, cx)` silencieuse, sans émission d'événement), pas l'appelant qu'il faut
contorsionner. Le test qui ancre le correctif : poser le focus sur l'entité,
provoquer le changement, puis vérifier **deux** choses — que l'entité est la même
(`assert_eq!(view.field, field)`) et que `focus_handle.is_focused(window)` tient
encore. La première explique la seconde quand elle casse.

Voir aussi [[gpui-debug-bounds-et-clics]].
