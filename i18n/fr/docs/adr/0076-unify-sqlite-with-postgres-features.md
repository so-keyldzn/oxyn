<!-- oxyn-translation source="docs/adr/0076-unify-sqlite-with-postgres-features.md" sha256="25b28c383c14" -->

> Traduction française de [l'ADR-0076](../../../../docs/adr/0076-unify-sqlite-with-postgres-features.md). **La version anglaise fait foi.**

# ADR-0076 — Unifier SQLite tout en activant les types PostgreSQL

**Statut :** accepté · **Date :** 2026-11-12

Précise [l'ADR-0010](0010-contraintes-natives-sqlite.md) : la contrainte d'un
seul SQLite natif demeure ; son ancien épinglage de version est remplacé.

## Contexte

SQLite 3.50.2 embarqué dans `libsqlite3-sys` 0.35 est affecté par
RUSTSEC-2026-0459, une divulgation potentielle de mémoire à la lecture d'un WAL
malveillant. `rusqlite` 0.41.1 embarque SQLite 3.54.1 par `libsqlite3-sys`
0.39.1. Les fonctionnalités `chrono`, `uuid` et `json` de la façade SQLx 0.9
résolvent aussi son paquet SQLite optionnel, dont la dépendance native ne peut
coexister avec cette version. Les vérifications datées figurent dans les
[notes de recherche](../RESEARCH-NOTES.md).

## Décision

Activer `chrono`, `uuid` et `json` sur la dépendance de workspace
`sqlx-postgres` 0.9.0 épinglée exactement et utilisée par le driver PostgreSQL,
plutôt que sur la façade SQLx. Conserver la façade SQLx pour les API du driver
et utiliser `rusqlite` 0.41.1 avec SQLite embarqué. L'unification des
fonctionnalités Cargo active les mêmes fonctionnalités SQLx core et PostgreSQL ;
les graphes de la machine hôte ont été comparés avant et après. Une seule
version de `libsqlite3-sys` est résolue.

## Conséquences

Le correctif SQLite est disponible sans changer l'API de la façade PostgreSQL
ni maintenir de fork. Le manifeste PostgreSQL dépend maintenant directement
d'un composant interne de SQLx, épinglé à la version de la façade ; les futures
montées de SQLx doivent vérifier les graphes de fonctionnalités et les
dépendances natives. La montée de rusqlite demande d'adapter les API faillibles
du gestionnaire de progression et des métadonnées. La comparaison des
fonctionnalités ne démontre pas l'équivalence à l'exécution sur toute plateforme.

## Solutions écartées

- Garder SQLite 3.50.2 : laisse l'alerte de divulgation sans correctif.
- Forker SQLx ou sa dépendance SQLite : ajoute une charge de maintenance et de sécurité.
- Utiliser SQLite système : abandonne la version embarquée reproductible sans
  supprimer le conflit de liaison native de Cargo.
- Remplacer la façade SQLx ou le driver PostgreSQL : modifie une surface d'API
  plus grande que nécessaire pour résoudre le conflit de fonctionnalités.

## Coût de sortie et réexamen

Revenir sur le câblage demande de coordonner les deux manifestes SQLx et de
vérifier les graphes de fonctionnalités et de dépendances natives ; changer le
driver ou remplacer SQLite coûte nettement plus. Réexaminer lorsque SQLx peut
activer les types PostgreSQL requis sans résoudre sa dépendance SQLite
optionnelle incompatible, ou lorsqu'une mise à jour change ces contraintes.
