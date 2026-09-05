# Fin de tâche

Ce qui doit être vrai avant d'annoncer qu'une tâche est terminée.

**Une tâche annoncée comme faite sans que `make qualite` soit passé est une tâche
non vérifiée.** Ne jamais annoncer vert un contrôle qui n'a pas été lancé.

## La porte

- [ ] `make qualite` passe — format, clippy en `-D warnings`, tests,
      documentation
- [ ] Aucun avertissement laissé « pour plus tard »

Si `Cargo.toml` n'existe pas encore, `make qualite` le **dit** et ne prétend pas
au succès complet. Lire sa sortie plutôt que son code de retour.

## Invariants

- [ ] `relecteur-invariants` lancé sur le changement, rien de bloquant
- [ ] Si une frontière externe est touchée : `relecteur-frontiere`
- [ ] Si des secrets, du `unsafe` ou l'IA sont touchés : `relecteur-securite`

## Documentation

- [ ] Aucun document de `docs/` rendu faux par ce changement — sinon c'est un bug
      à corriger dans le **même** commit
- [ ] Toute décision structurante prise en chemin a son ADR
- [ ] Toute version externe ajoutée est dans `docs/RESEARCH-NOTES.md`, avec sa
      date
- [ ] `make socle` passe

## Hygiène

- [ ] Aucun code mort, aucun code commenté « au cas où »
- [ ] Aucun `TODO` sans date ni sans ce qui le débloque
- [ ] Aucune abstraction pour un seul appelant
- [ ] Aucun module ou crate au nom fourre-tout
- [ ] Les commentaires disent *pourquoi*, pas ce que le code dit déjà

## Commit

- [ ] Format `type(portee): sujet`, en français, minuscule, sans point final,
      72 caractères maximum
- [ ] Si le changement est une optimisation : le chiffre avant et après est dans
      le message
- [ ] Aucun `--no-verify`

## Ce qu'il faut dire dans le rapport

- ce qui a été **réellement vérifié**, et par quelle commande ;
- ce qui reste ouvert : les doutes, les pièges soupçonnés sans être confirmés ;
- ce qui a été laissé de côté, et pourquoi.

Un doute signalé vaut mieux qu'une certitude fabriquée.
