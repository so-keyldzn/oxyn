---
name: piege-bassin-postgres-et-cache-sqlx
description: Comment forcer et observer plusieurs connexions du bassin PostgreSQL dans un test, et le cache d'instructions préparées de sqlx qui déplace l'erreur de la préparation vers le flux
metadata:
  type: reference
---

Pièges relevés le 2026-09-10 en éprouvant le contexte de session sur PostgreSQL
17.11. Ils portent sur `sqlx` et sur le cycle de vie d'une connexion, pas sur le
produit — le contrat vit dans `docs/`.

## Forcer plusieurs connexions du bassin

`execute` tient sa connexion du début à la fin et le canal du curseur est borné
à **un** lot. Tant qu'on ne draine pas, la tâche de flux reste bloquée sur sa
connexion. Donc : ouvrir `MAX_CONNECTIONS` curseurs **avant** d'en drainer un
seul, sur une requête qui dépasse `BATCH_ROW_CEILING` (8 192 lignes) — un
`CROSS JOIN generate_series(1, 50000)` suffit et coûte quelques millisecondes.
Relever `pg_backend_pid()` **dans la requête elle-même** pour le prouver.

## Une connexion abandonnée ne revient pas au bassin

Deux chemins la ferment (`close_on_drop`), et alors deux exécutions successives
ne tombent **jamais** sur le même backend :

- un curseur détruit avant que `next_batch()` ait rendu `None` — il a pu laisser
  des octets non lus ;
- un `prepare` qui échoue alors que `limits.read_only` est vrai.

Conséquence pour un test qui compare des pids d'une exécution à l'autre : drainer
jusqu'à `None`, et n'y mettre aucune requête qui échoue. Pour observer une
résolution *sans* échouer, `pg_catalog.to_regclass('nom')` emprunte le même
`search_path` et rend `NULL` au lieu de lever.

Corollaire : comparer **un** pid à **un** autre reste fragile — le bassin garde
plusieurs connexions inactives et ne les rend pas en FIFO. Ce qui tient, c'est de
saturer le bassin, de comparer l'**ensemble** des pids d'une phase à l'autre.

## Le rendu d'une définition dépend du `search_path`

Relevé sur 17.11 : `format_type`, `pg_get_constraintdef`, `pg_get_indexdef` et
`pg_get_expr` **qualifient** leur sortie quand le schéma n'est pas sur le chemin
et l'**omettent** quand il y est — `oxyn_ctx_a.ctx_amount` contre `ctx_amount`.

C'est le seul canal permettant d'observer le `search_path` d'une connexion du
bassin **sans** passer par une exécution (qui, elle, repose son `SET`) : une
fixture avec un domaine et une fonction dans le schéma, lue par le catalogue,
révèle l'état de la connexion qui a servi.

Pour rendre l'observation déterministe : le bassin est plafonné à quatre, donc
tenir trois curseurs vivants force le catalogue sur la quatrième connexion.

## Le cache d'instructions préparées de sqlx

`sqlx` garde un cache **par connexion**, indexé sur le texte SQL. Après un
`SET search_path`, réexécuter le même texte ne repasse pas par une préparation
côté client : `execute` rend `Ok` même si la relation n'est plus résoluble, et le
refus arrive **par le flux**. Dans un test, passer par `echouer` et non par
`refus`.

Vérifié sur 17.11 : le serveur refait bien son analyse à l'exécution quand
`search_path` a changé. Une table homonyme dans deux schémas rend les lignes du
**nouveau** schéma, pas celles du schéma de la préparation. C'est le comportement
qu'il faut re-vérifier avant de faire confiance à un cache d'instructions sur un
autre moteur.

Voir [[outil-cluster-postgres-jetable]] pour démarrer le serveur d'essai.
