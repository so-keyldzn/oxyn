---
name: gpui-simulate-keystrokes-et-focus
description: Piège GPUI en test — simulate_keystrokes ne déclenche aucun raccourci si rien n'est focalisé dans la vue ; l'assertion échoue sur l'état, pas sur la touche
metadata:
  type: feedback
---

`cx.simulate_keystrokes("cmd-j")` n'atteint un `on_key_down` posé sur la racine
d'une vue **que si le focus est déjà quelque part dans cette vue**. Sans focus,
la touche part dans le vide : aucune erreur, aucun avertissement, et le test
échoue plus loin sur l'état inchangé (`assert_eq!(view.panel, …)` — left:
`Object`, right: `Sql`), ce qui envoie chercher le défaut dans la logique du
raccourci au lieu du focus.

Le remède, avant la frappe :

```rust
cx.update(|window, cx| window.focus(&view.read(cx).focus_handle(cx)));
cx.run_until_parked();
```

**Why:** rencontré deux fois dans la même session en écrivant les tests du
rafraîchissement automatique. Un test manipulait l'arbre du catalogue
(`tree.update(…, select)`) puis frappait `cmd-j` : la sélection ne pose pas le
focus, donc le raccourci n'existait plus. Le second cas était `cmd-shift-h`
après une simple ouverture de fenêtre.

**How to apply:** dès qu'un test simule un raccourci **de vue** (pas une saisie
dans un champ déjà focalisé), poser explicitement le focus juste avant. Et
vérifier le modificateur dans le `match` du gestionnaire plutôt que de le
supposer : ici `("h", true)` voulait dire `cmd-shift-h`, pas `cmd-h`.

Voir aussi [[gpui-stop-propagation-et-raccourcis]], qui décrit l'autre moitié du
problème : le focus posé dans un champ qui, lui, **consomme** la touche.
