# Memory — driveriste

- [Disposable PostgreSQL cluster](outil-cluster-postgres-jetable.md) — recreate it (initdb, `-k ''`), `oxyn_test` role, the red tests that are not regressions
- [Mistargeted cancellation, reproducible window](piege-annulation-fenetre-deterministe.md) — per-connection `sqlite3_interrupt` flag, pid reused by the pool, advisory locks and `ClientWrite` in tests
- [PostgreSQL pool and sqlx cache](piege-bassin-postgres-et-cache-sqlx.md) — force several connections in a test, abandoned connection = closed connection, preparation served by the cache
- [Key-order cost, DuckDB and MongoDB](piege-ordre-par-cle-duckdb-mongo.md) — DuckDB TOP_N timings on 10M rows, ambiguous `idIndex` in listCollections docs
- [SQL composed around the user's text](piege-sql-compose-autour-du-texte-utilisateur.md) — newline against `--`, parentheses against an unclosed `/*`, and the `;` that really runs in SQLite
