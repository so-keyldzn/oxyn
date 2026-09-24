# Oxyn — Vision

> The modern database workspace.

Oxyn est un workspace de bureau, au backend natif et haute performance, pour explorer,
comprendre, interroger et gérer tout type de base de données. Un point d'accès unique aux
bases SQL, NoSQL, vectorielles, cloud et locales.

Au-delà d'un client de base de données traditionnel, Oxyn embarque des assistants IA
qui aident à comprendre les schémas, optimiser les requêtes, documenter les bases,
investiguer les incidents et répondre à des questions complexes sur les données.
Qu'ils soient servis par des modèles locaux ou des APIs cloud, les agents travaillent
*aux côtés* de l'utilisateur — jamais à sa place.

## Principes fondateurs

* **Native first** — un backend natif en Rust, aucun serveur, et une interface
  soumise à des budgets de trame chiffrés, qui se mesurent
  ([PERFORMANCE](PERFORMANCE.md)). La webview de Tauri affiche et
  saisit ; ce qui exécute une requête, parle à une base ou conserve un secret
  vit dans le processus Rust ([ADR-0029](adr/0029-interface-tauri-shadcn.md)).
* **Blazing fast** — la latence perçue est une fonctionnalité.
* **Open by default** — formats ouverts, pas de verrouillage.
* **AI when it adds value** — jamais imposée, jamais dans le chemin critique.
* **Privacy first** — hors-ligne par défaut, aucune donnée ne sort sans consentement.
* **Extensible through plugins**
* **Built for professionals**

## Bases de données visées

| Famille | Systèmes |
|---|---|
| Relationnel | PostgreSQL, MySQL, MariaDB, SQLite, SQL Server, Oracle |
| Analytique | DuckDB, ClickHouse, Snowflake, BigQuery, Redshift |
| NoSQL | MongoDB, Redis, Cassandra, DynamoDB, Couchbase |
| Vectoriel | pgvector, Milvus, Weaviate, Pinecone, Qdrant, ChromaDB |
| Graphe | Neo4j, Memgraph |
| Séries temporelles | TimescaleDB, InfluxDB |
| Recherche | Elasticsearch, OpenSearch |

## Workspace IA

Expliquer des schémas complexes · comprendre les relations entre tables · générer du SQL ·
améliorer les performances · détecter les anti-patterns · trouver les index manquants ·
relire les migrations · expliquer les plans d'exécution · repérer les tables inutilisées ·
détecter les données dupliquées · trouver les enregistrements incohérents · générer la
documentation · produire des diagrammes ER · construire des dictionnaires de données ·
répondre en langage naturel · suggérer des optimisations · assister le débogage ·
résumer de gros jeux de données · comparer des versions de base · expliquer les procédures
stockées · auditer permissions et sécurité.

## Architecture multi-agents

Agents spécialisés et collaboratifs : SQL · Schema · Performance · Migration · Security ·
Documentation · Data Quality · Analytics · Visualization.

## Fournisseurs IA

Local : Ollama, LM Studio, llama.cpp.
Cloud : OpenAI, Anthropic, Google Gemini, OpenRouter, Azure OpenAI, AWS Bedrock, APIs
compatibles OpenAI.

**Aucun fournisseur n'est requis. Tout doit fonctionner hors-ligne autant que possible.**

## Vision long terme

Devenir le *système d'exploitation des bases de données*. Pas seulement un client SQL.
Pas seulement un outil IA. Un workspace complet où humains et IA collaborent pour
comprendre, maintenir, optimiser et faire évoluer les systèmes de données modernes.
