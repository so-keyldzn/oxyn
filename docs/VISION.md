# Oxyn — Vision

> The modern database workspace.

Oxyn is a desktop workspace, with a native high-performance backend, to explore,
understand, query and manage any kind of database. A single access point to SQL,
NoSQL, vector, cloud and local databases.

Beyond a traditional database client, Oxyn ships AI assistants that help
understand schemas, optimize queries, document databases, investigate incidents
and answer complex questions about the data. Whether served by local models or
cloud APIs, the agents work *alongside* the user — never in their place.

## Founding principles

* **Native first** — a native Rust backend, no server, and an interface held to
  numeric frame budgets, which are measured
  ([PERFORMANCE](PERFORMANCE.md)). Tauri's webview displays and takes input;
  what runs a query, talks to a database or keeps a secret lives in the Rust
  process ([ADR-0029](adr/0029-interface-tauri-shadcn.md)).
* **Blazing fast** — perceived latency is a feature.
* **Open by default** — open formats, no lock-in.
* **AI when it adds value** — never imposed, never on the critical path.
* **Privacy first** — offline by default, no data leaves without consent.
* **Extensible through plugins**
* **Built for professionals**

## Target databases

| Family | Systems |
|---|---|
| Relational | PostgreSQL, MySQL, MariaDB, SQLite, SQL Server, Oracle |
| Analytical | DuckDB, ClickHouse, Snowflake, BigQuery, Redshift |
| NoSQL | MongoDB, Redis, Cassandra, DynamoDB, Couchbase |
| Vector | pgvector, Milvus, Weaviate, Pinecone, Qdrant, ChromaDB |
| Graph | Neo4j, Memgraph |
| Time series | TimescaleDB, InfluxDB |
| Search | Elasticsearch, OpenSearch |

## AI workspace

Explain complex schemas · understand relationships between tables · generate SQL ·
improve performance · detect anti-patterns · find missing indexes ·
review migrations · explain execution plans · spot unused tables ·
detect duplicated data · find inconsistent records · generate
documentation · produce ER diagrams · build data dictionaries ·
answer in natural language · suggest optimizations · assist debugging ·
summarize large datasets · compare database versions · explain stored
procedures · audit permissions and security.

## Multi-agent architecture

Specialized, collaborating agents: SQL · Schema · Performance · Migration · Security ·
Documentation · Data Quality · Analytics · Visualization.

## AI providers

Local: Ollama, LM Studio, llama.cpp.
Cloud: OpenAI, Anthropic, Google Gemini, OpenRouter, Azure OpenAI, AWS Bedrock,
OpenAI-compatible APIs.

**No provider is required. Everything must work offline as much as possible.**

## Long-term vision

Become the *operating system of databases*. Not just a SQL client.
Not just an AI tool. A complete workspace where humans and AI collaborate to
understand, maintain, optimize and evolve modern data systems.
