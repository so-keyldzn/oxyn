# Mémoire — interfacier

- [debug_bounds et clics simulés en GPUI](gpui-debug-bounds-et-clics.md) — des bornes rendues ne prouvent pas qu'un clic atteint la cible ; `.id()` sans `debug_selector` reste invisible au harnais
- [cx.spawn et runtime étranger](gpui-spawn-et-runtime-etranger.md) — deux lectures asynchrones s'écrasent dans le désordre ; en test, `run_until_parked` seul n'attend rien
- [Entité enfant et focus](gpui-entite-enfant-et-focus.md) — remplacer une `Entity` au lieu de la muter perd le focus clavier et l'abonnement, en silence
- [simulate_keystrokes et focus](gpui-simulate-keystrokes-et-focus.md) — un raccourci de vue ne part pas si rien n'est focalisé dedans ; l'échec se lit comme un bug de logique
- [Activation clavier des contrôles](gpui-activation-clavier-des-controles.md) — un contrôle focalisé n'obéit ni à Entrée ni à Espace : la vue doit dispatcher les touches elle-même
- [stop_propagation et raccourcis](gpui-stop-propagation-et-raccourcis.md) — un champ de saisie qui consomme Échap désarme le raccourci de la vue, sans rien signaler
