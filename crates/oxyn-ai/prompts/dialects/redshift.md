## The dialect: Amazon Redshift (`{{dialect}}`)

This connection reaches Redshift through Oxyn's `{{driver}}` driver: the protocol is PostgreSQL's, the grammar is not. Redshift forked from a very old PostgreSQL, and much that PostgreSQL added since is missing — `INSERT … ON CONFLICT`, `DISTINCT ON`, array columns among others. Do not carry a PostgreSQL habit over without knowing Redshift accepts it.

Writing it:
- Names fold to lower case, and by default quoted names fold too: `"Users"` and `users` are the same table unless the cluster enables case-sensitive identifiers. Spell names as the context does. Single quotes delimit strings.
- Limit rows with `LIMIT n OFFSET m`, or `SELECT TOP n`.
- To upsert, use `MERGE`, or a `DELETE` then an `INSERT` — each one a separate statement, each one approved by the user.
- Worth using: `GETDATE()`, `DATEADD`, `DATEDIFF`, `LISTAGG`, `APPROXIMATE COUNT(DISTINCT …)`, window functions, and PartiQL paths on `SUPER` columns. Integer division truncates (`1 / 2` is `0`).
- Primary, unique and foreign keys are **declared, not enforced**: duplicates and orphan rows can exist whatever the structure says, and the planner trusts the declaration anyway — a declared but violated key can return wrong results. Only `NOT NULL` holds. When a key matters to your answer, say so, and check with a read before relying on it.

Reading a plan:
- `EXPLAIN` shows the plan without running the statement; Redshift has no `EXPLAIN ANALYZE`. `ANALYZE` alone is another command: it recomputes a table's statistics, which is slow on a large table — never send it to look at a plan. You never see the plan; it goes to the user's result grid, so tell them what to look for.
- What to look for is data movement: `DS_DIST_NONE` and `DS_DIST_ALL_NONE` mean the join runs where the rows are; `DS_BCAST_INNER` copies the inner table to every node; `DS_DIST_BOTH` redistributes both sides — the costliest. A `Nested Loop` usually means a missing join condition.

How it performs:
- Storage is columnar and compressed: select only the columns you need.
- The distribution key decides where rows live. Joining two large tables on their distribution key avoids moving data; joining on anything else moves it. A skewed distribution key leaves one node doing the work.
- The sort key, through zone maps, lets a range filter on it skip blocks; a filter on an unsorted column reads every block. Deleted and updated rows stay as dead rows until `VACUUM`, and statistics drift until `ANALYZE`.
- Queries wait in workload-management queues: a heavy query delays others, and may be cancelled by a queue's limits.

Rules here:
- You cannot steer a transaction: Oxyn refuses `BEGIN`, `COMMIT`, `ROLLBACK` and `SAVEPOINT` from an agent, and every statement you send stands alone. `VACUUM` cannot run in a transaction anyway. Do not change settings with `SET`: Oxyn treats it as a write.
- `COPY` and `UNLOAD` read from or write to storage outside the database: propose one only when the user asks for it, with the location they give.

The structure lives in `SVV_TABLE_INFO` (distribution style, sort key, skew, unsorted share), `SVV_COLUMNS` and `PG_TABLE_DEF` (which only lists the schemas on the search path), but a query on them sends its rows to the user's grid, not to you. To learn the structure, call `describe_schema`. Some catalog functions run only on the leader node and fail when the same query reads a user table.
