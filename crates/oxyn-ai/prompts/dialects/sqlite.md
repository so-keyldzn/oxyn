## The dialect: SQLite (`{{dialect}}`)

This connection opens a SQLite file through Oxyn's `{{driver}}` driver, inside Oxyn's own process: there is no server, and the file is the whole database. Features depend on the library's version; when the context gives it, do not use what that version lacks (`RETURNING`, `STRICT` tables, `RIGHT` and `FULL OUTER JOIN`, `unixepoch()` are recent).

Writing it:
- Double quotes name an identifier, single quotes delimit a string. Names are case-insensitive for ASCII letters. Depending on how the library was built, a double-quoted word that matches no column is taken as a string literal: a misspelled name then fails silently, so spell names exactly as the context does.
- Limit rows with `LIMIT n OFFSET m`.
- Types are affinities, not constraints: unless the table is `STRICT`, an `INTEGER` column can hold text, and `'1'` and `1` do not compare equal in every context. Use `typeof(column)` to check what a column really holds. Booleans are `0` and `1`.
- There is no date type. Dates are text (ISO 8601), real (Julian day) or integer (Unix time), as the application wrote them: use `date()`, `datetime()`, `strftime()`, `julianday()` on what is actually stored, and compare ISO 8601 text only when every row uses the same format.
- `LIKE` ignores ASCII case and rarely uses an index; a prefix range (`>= 'abc' AND < 'abd'`) does. `GLOB` is case-sensitive.
- Worth using: `INSERT … ON CONFLICT … DO UPDATE`, window functions, `json_extract`, `||` to concatenate.

Reading a plan:
- `EXPLAIN QUERY PLAN <statement>` shows the plan without running the statement; plain `EXPLAIN` shows virtual-machine bytecode, not a plan. You never see either; they go to the user's result grid, so tell them what to look for.
- What to look for: `SCAN t` is a full scan, `SEARCH t USING INDEX` an index lookup, `USING COVERING INDEX` one that never reads the table, `USE TEMP B-TREE FOR ORDER BY` a sort the index could have avoided. Without `ANALYZE` statistics (`sqlite_stat1`), the planner guesses selectivity.

Storage and concurrency:
- One writer at a time for the whole file: a long write blocks every other writer, which waits or fails with `database is locked`. In WAL mode, readers continue during a write; otherwise they wait too. Batch a large write so it holds the lock briefly.
- A table is ordered by its `rowid`; an `INTEGER PRIMARY KEY` column is that `rowid`, any other primary key is a separate index. `WITHOUT ROWID` tables are ordered by their primary key.
- Foreign keys are enforced only on a connection that turned them on with `PRAGMA foreign_keys`; a declared key does not prove the rows obey it. Check with a read before relying on it.

Changing the structure:
- `ALTER TABLE` only renames a table, renames, adds or drops a column. Changing a column's type or constraints means creating a new table, copying the rows, dropping the old one and renaming the new one — say so, and propose the steps one statement at a time, each approved by the user; say what state the file is in if a step fails.
- DDL is transactional in SQLite, but Oxyn refuses `BEGIN`, `COMMIT`, `ROLLBACK` and `SAVEPOINT` from an agent: every statement you send stands alone. Do not send `PRAGMA` statements that change settings either: they change how the user's next statements behave.
- `VACUUM` rewrites the whole file, needs as much free disk again, and holds the write lock until it ends.

The structure lives in `sqlite_schema` and in `pragma_table_info('table')`, `pragma_index_list('table')`, `pragma_foreign_key_list('table')`, but a query on them sends its rows to the user's grid, not to you. To learn the structure, call `describe_schema`.
