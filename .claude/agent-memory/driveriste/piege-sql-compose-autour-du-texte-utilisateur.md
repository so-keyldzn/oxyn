---
name: piege-sql-compose-autour-du-texte-utilisateur
description: The two guards to put in place when a driver composes SQL around a fragment written by the user — newline against `--`, parentheses against an unclosed `/*` — and the `;` that really runs in SQLite but not in PostgreSQL
metadata:
  type: reference
---

Found on 2026-09-10 while composing `SELECT … WHERE <user's text>
ORDER BY … LIMIT n` (PostgreSQL 17, embedded SQLite). They concern the engines,
not the product.

## Two guards, and each catches what the other lets through

The fragment is neither parsed nor rewritten. What makes it safe is **the shape
of what surrounds it**:

* **a newline after the fragment**, because a trailing `-- …` would comment out
  the rest of the statement — `ORDER BY` and `LIMIT` included — and turn a
  bounded read into a full scan, with no error anywhere;
* **parentheses around the fragment**, because no newline terminates an
  unclosed `/*`. Checked: `SELECT a FROM t WHERE a>0 /*⏎LIMIT 1` returns **all**
  rows in SQLite (PostgreSQL refuses it). With `WHERE (a>0 /*⏎) LIMIT 1`, the
  closing parenthesis is swallowed, the statement becomes incomplete and the
  engine refuses it. `WHERE (X)` equals `WHERE X` for any boolean expression:
  nothing legitimate changes meaning, including an already parenthesized
  fragment.

The order of the two matters: the closing parenthesis must be **on the next
line**, otherwise it falls into the `--`.

## A `;` in the fragment does not behave the same on both sides

* **PostgreSQL, extended protocol**: `prepare` refuses several statements
  ("cannot insert multiple commands into a prepared statement"). The second one
  never reaches execution.
* **SQLite**: the text is a *batch*. Each statement is really prepared and
  executed one after the other; what stops a `DROP` slipped in after a `;` is
  the `sqlite3_stmt_readonly` check before execution, not the splitting. Without
  read-only limits, it would run.

Consequence for a test: on SQLite, the useful proof is a **witness table that
still exists at the end**, not just a returned error.

See [[outil-cluster-postgres-jetable]] for the test server.
