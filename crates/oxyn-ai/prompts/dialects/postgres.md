## The dialect: PostgreSQL (`{{dialect}}`)

This connection speaks PostgreSQL through Oxyn's `{{driver}}` driver. Servers and extensions built on it — TimescaleDB, Citus, pgvector — follow what is below; Redshift is a different dialect. Features depend on the server version: when the context gives it, do not use what that version lacks (`MERGE`, `NULLS NOT DISTINCT`, SQL/JSON functions are recent).

Writing it:
- An unquoted name folds to lower case: `Users` and `users` are the same table, `"Users"` is another one. Double-quote a name that has upper case, a space or a reserved word, exactly as the context spells it. Single quotes delimit strings.
- Limit rows with `LIMIT n OFFSET m` or `FETCH FIRST n ROWS ONLY`. There is no `TOP`.
- Worth using: `DISTINCT ON (…)` for the first row per group, `LATERAL` for a top-n per row, `FILTER (WHERE …)` on aggregates, `RETURNING`, `INSERT … ON CONFLICT … DO UPDATE`, `IS DISTINCT FROM`, `= ANY(array)`, and `->`, `->>`, `@>` on `jsonb` (`@>` can use a GIN index, `->>` comparisons cannot).
- `timestamp` has no time zone and `timestamptz` does: comparing them converts with the session's `TimeZone`. Integer division truncates (`1 / 2` is `0`).

Reading a plan:
- `EXPLAIN` shows the plan without running the statement. `EXPLAIN ANALYZE`, and `EXPLAIN (ANALYZE, BUFFERS)`, **run** it: on an `UPDATE` or a `DELETE`, the rows really change, so Oxyn treats it as the write it runs. For a write, propose plain `EXPLAIN`. You never see the plan; it goes to the user's result grid, so tell them what to look for.
- What to look for: estimated against actual rows on the lowest node where they diverge — a misestimate of ten times or more drives a wrong join method; it comes from stale statistics or correlated columns (`CREATE STATISTICS`). A `Seq Scan` is right on a small table or an unselective filter; a `Nested Loop` over many outer rows is not. `Rows Removed by Filter` large against rows returned means a missing or unusable index. `shared read` against `shared hit` tells cold cache from a bad plan.

Indexes:
- A B-tree serves equality and ranges on its leading columns, in order. `lower(email) = …` needs an expression index on `lower(email)`; `LIKE 'abc%'` needs `text_pattern_ops` unless the collation is `C`; `ILIKE '%abc%'` needs a trigram GIN index (`pg_trgm`). A partial index (`WHERE deleted_at IS NULL`) fits a hot subset.
- The referencing side of a foreign key is **not** indexed automatically: a delete on the parent then scans the child.

Changing the structure:
- DDL is transactional in PostgreSQL, but you cannot use that here: Oxyn refuses `BEGIN`, `COMMIT`, `ROLLBACK` and `SAVEPOINT` from an agent, and every statement you send stands alone.
- Most `ALTER TABLE` forms take an `ACCESS EXCLUSIVE` lock: the statement waits behind every open transaction on the table, and every query behind it waits too. On a busy table, tell the user to set a short `lock_timeout` in their own session first.
- `CREATE INDEX` blocks writes for its whole duration. `CREATE INDEX CONCURRENTLY` does not, but cannot run in a transaction and leaves an `INVALID` index behind if it fails — which must be dropped.
- Adding a column with a constant default is a catalog change; a volatile default (`now()` is not, `random()` is), or changing a column's type, rewrites the table under its lock. Add a foreign key or a check as `NOT VALID`, then `VALIDATE CONSTRAINT` in a second statement, to avoid scanning the table under the strong lock.
- Do not change `search_path` or any other setting with `SET`: it changes which table the user's next statement reaches, so Oxyn treats it as a write, and refuses `SET ROLE` outright. Qualify names with their schema instead.

Data at scale:
- `count(*)` on a large table reads all of it (MVCC keeps no row count); if an estimate is enough, say that `pg_class.reltuples` holds one.
- A large `UPDATE` writes a new version of every row it touches and leaves the old ones for `VACUUM`: batch it by key range, and say the table will bloat until vacuumed.

The structure lives in `pg_catalog` and `information_schema`, but a query on them sends its rows to the user's grid, not to you. To learn the structure, call `describe_schema`.
