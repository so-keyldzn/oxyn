---
name: gpui-activation-clavier-des-controles
description: En GPUI 0.2.2, un élément focalisé avec tab_index/tab_stop n'est PAS activé par Entrée ni Espace — il faut dispatcher les touches soi-même sur des FocusHandle
metadata:
  type: feedback
---

Un `div().tab_index(0).tab_stop(true).on_click(…)` qui a le focus **ne
déclenche pas** son `on_click` sur `Entrée` ni sur `Espace`, que le focus vienne
de `window.focus(&handle)` ou d'une tabulation. Un bouton dessiné ainsi est
donc atteignable et **inerte** au clavier.

**Why:** sonde écrite en session (2026-09-10, GPUI 0.2.2, harnais
`test-support`) : après `window.focus(&cancel_focus)`, `simulate_keystrokes`
sur `space` puis `enter` ne produit aucun effet ; quatre `tab` suivis d'`enter`
non plus. Aucun correctif amont n'est à attendre (ADR-0009), et la régression
est **silencieuse** : l'écran reste beau, la souris marche, le clavier non.

**How to apply:** dans toute vue nouvelle, garder une `FocusHandle` par geste
actionnable (ou une poignée de liste + un rang courant quand les lignes sont des
données renouvelées), la poser avec `.track_focus(&handle)` sur le contrôle, et
dispatcher `enter`/`space` dans le `on_key_down` de la vue selon
`handle.is_focused(window)`. Deux réserves qui se paient cher :

- rendre la main tout de suite si un champ de saisie tient le curseur, sinon la
  barre d'espace tapée dans un nom déclenche le bouton voisin ;
- ne pas « corriger » le composant de contrôle partagé pour activer sur
  `Entrée` : les écrans où `Entrée` **doit** rester inerte (confirmation d'une
  action destructrice) reposent sur ce silence.

Test qui rougit : focaliser la poignée, `simulate_keystrokes("enter")`,
affirmer l'effet. Voir aussi [[gpui-simulate-keystrokes-et-focus]] et
[[gpui-stop-propagation-et-raccourcis]].
