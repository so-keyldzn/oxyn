---
name: piege-sql-compose-autour-du-texte-utilisateur
description: Les deux gardes à poser quand un driver compose du SQL autour d'un fragment écrit par l'utilisateur — saut de ligne contre le `--`, parenthèses contre le `/*` non fermé — et le `;` qui s'exécute vraiment en SQLite mais pas en PostgreSQL
metadata:
  type: reference
---

Relevés le 2026-09-10 en composant `SELECT … WHERE <texte de l'utilisateur>
ORDER BY … LIMIT n` (PostgreSQL 17, SQLite embarqué). Ils portent sur les
moteurs, pas sur le produit.

## Deux gardes, et chacune rattrape ce que l'autre laisse passer

Le fragment n'est ni analysé ni réécrit. Ce qui le rend sûr, c'est **la forme de
ce qui l'entoure** :

* **un saut de ligne après le fragment**, parce qu'un `-- …` final commenterait
  le reste de l'instruction — `ORDER BY` et `LIMIT` compris — et transformerait
  une lecture bornée en balayage complet, sans erreur nulle part ;
* **des parenthèses autour du fragment**, parce qu'aucun saut de ligne ne
  termine un `/*` non fermé. Vérifié : `SELECT a FROM t WHERE a>0 /*⏎LIMIT 1`
  rend **toutes** les lignes en SQLite (PostgreSQL, lui, refuse). Avec
  `WHERE (a>0 /*⏎) LIMIT 1`, la parenthèse fermante est avalée, l'instruction
  devient incomplète et le moteur la refuse. `WHERE (X)` vaut `WHERE X` pour
  toute expression booléenne : rien de légitime ne change de sens, y compris un
  fragment déjà parenthésé.

L'ordre des deux compte : la parenthèse fermante doit être **sur la ligne
suivante**, sinon elle tombe dans le `--`.

## Un `;` dans le fragment ne se comporte pas pareil des deux côtés

* **PostgreSQL, protocole étendu** : `prepare` refuse plusieurs instructions
  (« cannot insert multiple commands into a prepared statement »). La seconde
  n'atteint jamais l'exécution.
* **SQLite** : le texte est un *lot*. Chaque instruction est réellement
  préparée et exécutée l'une après l'autre ; ce qui arrête un `DROP` glissé
  après un `;`, c'est le contrôle `sqlite3_stmt_readonly` avant exécution, pas
  le découpage. Sans limites en lecture seule, il tournerait.

Conséquence pour un test : sur SQLite, la preuve utile est une **table témoin
qui existe encore à la fin**, pas seulement une erreur retournée.

Voir [[outil-cluster-postgres-jetable]] pour le serveur d'essai.
