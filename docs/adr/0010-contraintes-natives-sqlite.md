# ADR-0010 — Une seule version de libsqlite3-sys dans le graphe

**Statut :** accepté · **Date :** 2026-09-05
**Découvert à :** la première résolution réelle du workspace, pas à la conception.

## Contexte

Oxyn a besoin de SQLite deux fois, pour deux raisons sans rapport :
`oxyn-store` l'utilise comme format d'état local (historique, journal d'audit, cache de
catalogue) via `rusqlite`, et `oxyn-driver-sqlite` l'expose comme base de données cliente.
Par ailleurs `oxyn-driver-postgres` dépend de `sqlx`.

Trois faits se combinent mal :

1. Cargo verrouille les dépendances **optionnelles** dans `Cargo.lock` : `sqlx` fait donc
   entrer `sqlx-sqlite` dans le graphe de résolution même avec `default-features = false`
   et sans aucune feature SQLite activée.
2. `sqlx-sqlite` 0.9 accepte `libsqlite3-sys >=0.30.1, <0.38` ; `rusqlite` 0.40 exige `^0.38`.
3. `libsqlite3-sys` déclare `links = "sqlite3"`. Cargo n'autorise **qu'un seul** paquet
   déclarant un `links` donné dans tout le graphe.

L'intersection des bornes est vide : le workspace ne résout pas.

## Décision

Épingler **`rusqlite` 0.37** (qui demande `libsqlite3-sys ^0.35`), seule version dont la
borne intersecte celle de `sqlx-sqlite`. Le graphe résout alors sur `libsqlite3-sys` 0.35,
partagé.

## Conséquences

* **+** Le workspace résout, et une seule copie de SQLite est compilée et liée.
* **−** `rusqlite` est bloqué trois versions mineures en arrière, sur une contrainte qui ne
  vient pas de lui. Toute API `rusqlite` postérieure à 0.37 est hors de portée.
* **−** La contrainte est **transitive et invisible** : elle ne vient d'aucune décision
  d'architecture, seulement de la coexistence de deux bibliothèques. Elle doit être écrite
  quelque part, sinon quelqu'un remontera `rusqlite` dans six mois et passera une soirée
  sur un message d'erreur `links` que rien n'explique.

## Alternatives écartées

| Alternative | Raison du rejet |
|---|---|
| Utiliser `sqlx-sqlite` partout, supprimer `rusqlite` | `oxyn-store` a besoin d'un accès synchrone simple ; passer par un runtime async pour lire l'historique local est un coût permanent pour éviter un épinglage temporaire |
| Retirer `sqlx` et écrire le driver PostgreSQL sur `tokio-postgres` | possible, mais on perd le pool, la gestion TLS et le typage de `sqlx` pour un problème de version |
| Compiler SQLite en non-bundled | déplace le problème vers la machine de l'utilisateur, et « native first » n'est pas « dépend de ce qui traîne sur le système » |

**Reconsidérer quand** `sqlx` élargira sa borne sur `libsqlite3-sys`. Ce jour-là,
remonter `rusqlite` est un changement d'une ligne — à condition que cet ADR ait été lu.
