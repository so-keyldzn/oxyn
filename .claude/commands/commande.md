---
description: Ajouter une commande au command bus
argument-hint: "<ce que la commande doit faire>"
allowed-tools: Bash, Read, Write, Edit, Grep, Glob
---

Objet : ajouter au command bus la commande qui permet **$ARGUMENTS**.

## Pourquoi ce geste a sa propre commande

[ADR-0004](../../docs/adr/0004-command-bus.md) est la décision la plus
structurante du projet et la plus facile à éroder. Il suffit qu'une vue appelle
un driver « en attendant », et le second chemin d'exécution existe pour toujours
— non audité, non journalisé, et c'est celui que l'IA empruntera.

**Une fonctionnalité commence par une commande, pas par une vue.**

## Avant d'écrire

1. `docs/adr/0004-command-bus.md`, en entier.
2. `docs/ARCHITECTURE.md` § le command bus.
3. Vérifier qu'aucune commande existante ne fait déjà ceci : deux commandes
   voisines divergent, et la politique appliquée à l'une finit par ne plus
   l'être à l'autre.

## Les quatre questions à trancher avant de coder

| Question | Pourquoi elle vient avant le code |
|---|---|
| Que fait la commande, exprimé en **une** phrase ? | une commande qui demande deux phrases en est deux |
| Est-elle une **lecture** ou une **écriture** ? | c'est ce que lit le `PolicyGate` ; s'il faut hésiter, c'est une écriture |
| Que décide le `PolicyGate` pour `Actor::Agent` ? | la politique par défaut est dans l'ADR-0004 ; toute exception se justifie ici |
| Est-elle **annulable**, et l'annulation atteint-elle le serveur ? | sinon l'interface ne pourra pas offrir un bouton honnête |

## Constructions à utiliser / jamais

| Utiliser | Jamais | Pourquoi |
|---|---|---|
| Une valeur `Command` typée | une chaîne, un nom d'action | le typage est ce qui rend le bus auditable |
| `Actor` porté par la commande | un drapeau global « mode agent » | un état global se désynchronise, et l'audit ment |
| Décision rendue par le `PolicyGate` | un `if` de vérification dans l'appelant | deux endroits qui décident, c'est un endroit qui oubliera |
| Un enregistrement d'audit par commande | journaliser dans la vue | la vue n'est pas le seul émetteur |

## Les pièges

**La commande « pratique » qui en emballe trois.** Elle contourne la politique :
le `PolicyGate` ne peut décider que sur ce qu'il voit, et il voit une lecture là
où il y a une écriture cachée.

**La lecture qui écrit.** `EXPLAIN ANALYZE` exécute réellement la requête
analysée, `DELETE` compris. Une vue matérialisée se rafraîchit. Une fonction
appelée dans un `SELECT` peut écrire. Classer sur le nom SQL est faux : classer
sur l'effet.

**L'audit après la décision.** Enregistrer seulement ce qui a été exécuté rend
invisible ce qui a été refusé — donc invisibles les tentatives répétées d'un
agent, qui sont précisément le signal qu'on voudrait voir.

## Vérifier

```bash
make qualite
```

Le test qui compte : la même commande émise avec `Actor::Human` et
`Actor::Agent` donne deux décisions du `PolicyGate`, et celle de l'agent est au
moins aussi restrictive.

## Rappels

- pas de second chemin, jamais, même temporaire ([I-01](../../CLAUDE.md#i-01)) ;
- sur une connexion `production`, un agent est en lecture seule stricte — c'est
  un refus, pas une confirmation renforcée : une confirmation finit par être
  cliquée.
