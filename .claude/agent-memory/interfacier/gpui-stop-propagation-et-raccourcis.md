---
name: gpui-stop-propagation-et-raccourcis
description: Piège GPUI — un composant de saisie qui appelle cx.stop_propagation() sur une touche désarme silencieusement le raccourci de fenêtre correspondant ; vérifier Échap à chaque champ ajouté
metadata:
  type: feedback
---

Sous GPUI, un raccourci posé par `on_key_down` sur la racine d'une vue ne se
déclenche qu'en remontée. Un composant de saisie qui traite la touche et appelle
`cx.stop_propagation()` **supprime donc le raccourci** tant qu'il a le focus —
sans erreur, sans avertissement, et sans que rien ne change au rendu.

Le cas concret : `oxyn_ui::TextField` traite `escape` (il émet `FieldEvent::Escape`
puis arrête la propagation). Poser ce champ dans une vue dont l'état « en cours »
s'annule par Échap fait disparaître **le seul moyen d'annuler** pour l'utilisateur
qui a le curseur dans le champ. Même mécanique pour `enter`, `tab` non géré,
`cmd-a`.

**Why:** rencontré en raccordant le champ `WHERE` de l'aperçu de table. Le
raccourci Échap de `on_workspace_key` annule la lecture en cours ; il ne
s'exécutait plus dès que le focus était dans le champ de filtre. La règle
« l'état en cours porte toujours un moyen d'annuler » était tenue au rendu et
fausse à l'usage.

**How to apply:** à chaque champ de saisie ajouté dans une vue, lister les
touches que le composant consomme (lire son `fn key`) et les croiser avec les
raccourcis de la vue. Ce qui est consommé se rebranche par **abonnement à
l'événement du composant** (`FieldEvent::Escape` → `cancel_…`), pas en
contournant la propagation. Le test qui l'ancre : poser le focus sur le champ,
`simulate_keystrokes("escape")`, puis vérifier que le `CancelToken` de la
requête est bien annulé — l'assertion doit porter sur le jeton, pas sur
l'affichage.

Voir aussi [[gpui-debug-bounds-et-clics]].
