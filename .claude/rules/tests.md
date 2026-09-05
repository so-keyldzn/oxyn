---
paths:
  - "**/tests/**"
  - "**/benches/**"
  - "**/*_test.rs"
---

# Tests et bancs d'essai — conventions

## Ce qu'on teste ici, et ce qu'on ne teste pas

| Teste | Ne teste pas |
|---|---|
| Le contrat : classification d'erreurs, capacités, conversion de types | que `sqlx` sait parler à PostgreSQL |
| Ce qui panique aujourd'hui si l'entrée est hostile | des accesseurs |
| Les cas de perte documentés de la table de types | le chemin nominal seul |

Un test qui ne peut pas échouer ne prouve rien et coûte à chaque exécution.

## Les entrées hostiles sont le sujet, pas un bonus

Un test de driver qui n'envoie que des réponses bien formées ne teste pas
[I-09](../../CLAUDE.md#i-09). Le corpus minimal : type inconnu, `NULL` sur une
colonne `NOT NULL`, entier hors bornes, encodage invalide, réponse tronquée en
plein flux, nom d'objet contenant un guillemet ou un point-virgule.

Ce dernier point est le plus négligé : une table nommée
`"users"; DROP TABLE audit; --` est légale dans PostgreSQL, et c'est le test qui
prouve [I-10](../../CLAUDE.md#i-10).

## Les deux tests qu'un driver ne contourne pas

1. **L'annulation atteint le serveur.** Vérifiée côté serveur — la vue des
   processus, pas le retour de la fonction. Un test qui vérifie que le futur
   s'est arrêté ne teste rien.
2. **Le flux tient sur un volume qui ne rentre pas en mémoire.** Avec une borne
   sur la mémoire du processus, sinon le test passe par accident sur une machine
   de développement bien dotée.

## Bancs d'essai

`criterion` pour le code pur : conversion vers `RecordBatch`, analyse,
formatage, diff de schéma
([PERFORMANCE](../../docs/PERFORMANCE.md#ce-qui-se-mesure-et-comment)).

Deux choses qui ne sont **pas** des bancs d'essai :

- une mesure contre une base réelle — le réseau et l'état du serveur dominent le
  signal ; ce qui se mesure, c'est le temps passé *dans* Oxyn ;
- un banc `criterion` sur du GPUI — il ne mesure rien d'utile ; les budgets de
  trame se mesurent avec les instruments du système.

Une optimisation arrive avec son chiffre avant et après, dans le message de
commit ([`/benchmark`](../commands/benchmark.md)).

## Ce qui ne va pas dans un test

Un identifiant réel, une chaîne de connexion, un jeton
([I-03](../../CLAUDE.md#i-03)) — y compris dans une fixture « de test » : elle
sera commitée, et elle est souvent réelle.
