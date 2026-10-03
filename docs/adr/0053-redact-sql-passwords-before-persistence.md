# ADR-0053 — Redact SQL password literals before persistence

**Status:** accepted · **Date:** 2026-10-03

## Context

Database administration statements can carry a password as a SQL string
literal. PostgreSQL `CREATE`/`ALTER ROLE` and `USER`, and MySQL
`CREATE`/`ALTER USER` and `SET PASSWORD`, therefore place a credential directly
in the statement text rather than in a bound value.

Oxyn persists statement text in `query_history`, the append-only
`audit_journal`, and conversation tool-call records. Keeping that text verbatim
would put a password in the workspace file, contrary to I-03 and
[SECURITY](../SECURITY.md#what-never-touches-the-disk-in-clear). Rewriting
existing `audit_journal` rows would contradict the journal's append-only
contract and destroy the exact historical bytes after the fact.

## Decision

`oxyn-query::redact_password_literals` uses the existing dialect-aware lexical
scanner to replace the string literal in `PASSWORD`, `IDENTIFIED BY`, and
`IDENTIFIED WITH … BY` clauses with `'<redacted>'`. It preserves all other
bytes, including formatting and comments.

The grammar references were checked on 2026-10-03 and are recorded in
[RESEARCH-NOTES](../RESEARCH-NOTES.md#sql-password-literals-before-persistence).
Quoted account names in `SET PASSWORD FOR` are preserved; the literal after
the assignment is redacted. Adjacent literal fragments and the old password
in `REPLACE` are redacted too. Where a record has no dialect or SQL mode,
the scanner combines the supported lexical interpretations conservatively;
ambiguous quoting can therefore redact more text than strictly necessary.

The original `Command` and `ExecRequest` remain unchanged and are sent to the
driver exactly as entered. `HistoryRecord`, `JournalRecord`, and serialized
`ToolCallRecord` values receive only the redacted copy at their persistence
boundaries. Stored history tells the reader that password literals are
redacted.

Saved-query mentions pass through the same function at the AI context gateway,
before clipping. This change covers recognized password clauses, not arbitrary
secrets in comments, dynamic SQL, editor drafts, or free-form conversation text.

The rule applies to new writes only. Existing `audit_journal` rows are not
rewritten, deleted, or migrated: append-only means that even a security repair
does not create a privileged path that can alter the audit trail. Users who may
have run credential-bearing SQL before this decision must rotate those
credentials and protect or replace the affected workspace file.

## Consequences

* **+** New history, journal, and tool-call rows do not retain recognized SQL password literals.
* **+** Execution semantics do not change because the driver receives the original request.
* **+** The audit journal remains strictly append-only, without a maintenance exception that could later erase evidence.
* **−** A stored statement is no longer byte-for-byte identical to the credential-bearing statement that ran.
* **−** Previously persisted credentials remain in old workspace files and backups; remediation requires credential rotation and file handling outside Oxyn.
* **−** A newly introduced database-specific password grammar needs an explicit lexer test before it is covered.

**Exit cost:** removing the rule requires changing the three persistence
boundaries, the history disclosure, and the security contract. Recovering
already redacted literals is impossible by design.

**Reconsider if** a future workspace format can cryptographically separate
secret-bearing audit evidence while remaining openly readable under I-11, or a
database dialect introduces credential syntax the lexical rule cannot identify
without ambiguity.

## Rejected alternatives

| Alternative | Reason for rejection |
|---|---|
| Rewrite existing journal rows | It violates the append-only audit contract and creates an alteration path whose abuse would be indistinguishable from remediation. |
| Redact with regular expressions | SQL comments, quoted identifiers, escaping, and batches make a regex both over-redact ordinary literals and miss valid credential syntax. |
| Redact the command before execution | It would change the password sent to the server and break the user's operation. |
| Stop persisting all statement text | History and audit would no longer answer what was run, and unrelated SQL is not secret by default. |
