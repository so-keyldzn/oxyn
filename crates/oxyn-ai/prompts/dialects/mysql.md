## The dialect: MySQL and MariaDB (`{{dialect}}`)

This connection speaks MySQL through Oxyn's `{{driver}}` driver, which also serves MariaDB. The two share most of the grammar and have drifted apart in places: when the context gives the server's product and version, write for that one; where they differ, the server's error says so — follow it rather than guessing.

Writing it:
- Backticks name an identifier: `` `order` ``, `` `user name` ``. Double quotes delimit a **string** unless the server runs with `ANSI_QUOTES`, so never use them for a name. Whether table names are case-sensitive depends on the server's platform and `lower_case_table_names`: spell them exactly as the context does.
- Limit rows with `LIMIT n OFFSET m`. MySQL has no `TOP` and no `FETCH FIRST`.
- Comparing a string column to a number converts the **column**: the index is not used, and `'abc' = 0` is true. Quote the literal to the column's type.
- String comparison follows the collation, usually case- and accent-insensitive (`_ci`, `_ai`): `'abc' = 'ABC'` is true, and a join between columns of different collations or charsets cannot use the index. `utf8` is the three-byte `utf8mb3`, which cannot hold every character; `utf8mb4` can.
- `TIMESTAMP` is stored in UTC and converted with the session's `time_zone`; `DATETIME` is stored as written. With `ONLY_FULL_GROUP_BY` on, every selected column must be grouped or aggregated.
- There is no `FULL OUTER JOIN`: write the `LEFT JOIN`, then `UNION ALL` the `RIGHT JOIN` filtered to the rows whose left key `IS NULL`. A plain `UNION` would also drop legitimate duplicate rows.
- To upsert, use `INSERT … ON DUPLICATE KEY UPDATE`. `REPLACE` deletes the old row and inserts a new one: delete triggers and `ON DELETE CASCADE` fire.

Reading a plan:
- `EXPLAIN` shows the plan without running the statement; `EXPLAIN FORMAT=TREE` is easier to read on MySQL. `EXPLAIN ANALYZE` on MySQL and `ANALYZE <statement>` on MariaDB **run** the statement: on an `UPDATE` or a `DELETE`, the rows really change, and Oxyn holds it like the write it runs. You never see the plan; it goes to the user's result grid, so tell them what to look for.
- What to look for: `type: ALL` is a full scan, `key: NULL` means no index was chosen, `rows` × `filtered` is the estimate of rows kept, `Using filesort` and `Using temporary` mean a sort or a temporary table the index could have avoided.

Indexes and InnoDB:
- InnoDB stores rows in primary-key order, and every secondary index carries the primary key: a wide primary key makes every index wide, and a random one (a UUID as text) scatters inserts. A composite index serves its leading columns, in order, up to the first range.
- At the default `REPEATABLE READ`, an `UPDATE` or `DELETE` locks the index ranges it scans, gaps included: a `WHERE` with no usable index locks most of the table for the statement's duration.
- `UPDATE … LIMIT` and `DELETE … LIMIT` without `ORDER BY` touch arbitrary rows; with an `ORDER BY` on the primary key, they make a sound batch.

Changing the structure:
- DDL is **not** transactional: `CREATE`, `ALTER`, `DROP`, `RENAME` and `TRUNCATE` commit at once and cannot be undone. Say so when you propose one. Besides, Oxyn refuses `BEGIN`, `START TRANSACTION`, `COMMIT`, `ROLLBACK` and `SAVEPOINT` from an agent: every statement you send stands alone.
- `ALTER TABLE` takes a metadata lock: it waits behind every open transaction that touched the table, and every new query on the table waits behind it. Write the algorithm you expect — `ALGORITHM=INSTANT`, or `ALGORITHM=INPLACE, LOCK=NONE` — so that the server refuses the change instead of silently copying the table under a lock. Which changes are instant depends on the version.
- Do not change settings or the default database with `SET` or `USE`: it changes what the user's next statement reaches. Qualify names with their database instead.

The structure lives in `information_schema` (`TABLES`, `COLUMNS`, `KEY_COLUMN_USAGE`, `STATISTICS`) and in `SHOW CREATE TABLE`, but a query on them sends its rows to the user's grid, not to you. To learn the structure, call `describe_schema`.
