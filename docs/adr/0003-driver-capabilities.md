# ADR-0003 — Modèle de capacités plutôt que dénominateur commun

**Statut :** accepté · **Date :** 2026-09-05

## Contexte
Redis n'a pas de schéma, Neo4j pas de tables, Elasticsearch pas de SQL, DynamoDB pas de
jointures. Une abstraction nivelante réduirait chaque base à sa plus pauvre expression.

## Décision
Chaque `Driver` et chaque `Session` déclarent un `Capabilities: u64` (bitflags). L'UI et
les agents interrogent ces drapeaux pour décider quelles surfaces exister. Les requêtes
portent un `QueryLanguage` explicite ; le SQL n'est qu'un cas parmi d'autres.

## Conséquences
* **+** Chaque base est exposée avec ses forces propres, rien n'est simulé.
* **+** Les agents ne proposent pas d'actions impossibles (pas d'index pour DynamoDB).
* **−** L'UI doit être conditionnelle partout : discipline à tenir dès la phase 0.
* Les capacités sont évaluées **par session**, pas par driver : la version du serveur
  change ce qui est disponible.

## Corollaire
Un driver par **protocole**, pas par produit : Redshift ≡ PostgreSQL, MariaDB ≡ MySQL,
OpenSearch ≡ Elasticsearch, Memgraph ≡ Bolt, pgvector et TimescaleDB ≡ extensions
PostgreSQL. Les ~30 systèmes de la vision se ramènent à ~14 implémentations.
