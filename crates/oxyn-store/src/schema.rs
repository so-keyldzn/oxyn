//! The local state schema and its migrations.
//!
//! Migrations are numbered SQL constants, applied **each in its own
//! transaction**, and recorded in `schema_version`. A migration that fails
//! leaves the schema exactly as it found it: SQLite can roll back DDL, unlike
//! most of its competitors, and that is what makes this strategy workable
//! without a recovery script.
//!
//! **A published migration is never rewritten.** It has already run on
//! someone's machine; changing it makes two installations reporting the same
//! schema number diverge. A new one is added instead.
//!
//! # The tables
//!
//! | Table | What it holds | Erasable |
//! |---|---|---|
//! | `workspaces` | the unit of persistence of user state | yes |
//! | `connections` | connection metadata — **never a secret** | yes |
//! | `query_history` | what the user ran | yes, explicit purge |
//! | `audit_journal` | the audit trail — **append-only** | **no** |
//! | `documents` | saved tabs and queries | yes |
//! | `workspace_preferences` | versioned display preferences | yes |
//! | `app_sessions` | what tells a clean shutdown from a crash | yes |
//! | `ai_providers` | the declared providers — **per machine** | yes |
//! | `external_agents` | the declared external agents — **per machine**, without a secret | yes |
//! | `ai_conversations` | the assistant's threads, per connection | yes, pruning and deletion |
//! | `ai_conversation_turns` | their transcript, **never** a database value | yes, with their thread |
//! | `ai_conversation_nodes` | the exchange tree; a withheld exchange only keeps its question, and its mentions as JSON — names and addresses, never a value | yes, with their thread |
//! | `ai_egress` | what left for an AI recipient — names, never values — **append-only** | **no** |
//!
//! # Why `STRICT`
//!
//! Without `STRICT`, SQLite happily stores the string `"demain"` in an
//! `INTEGER` column: type affinity is only a preference. An audit journal
//! whose columns accept anything is not an audit trail. `STRICT` requires
//! SQLite ≥ 3.37; the version embedded by `libsqlite3-sys` 0.35 (`bundled`
//! feature, ADR-0010) is 3.50.2.
//!
//! # What the tamper-proofing trigger covers, and what it does not
//!
//! Two `BEFORE UPDATE` and `BEFORE DELETE` triggers on `audit_journal` abort
//! any attempt, **including from the command-line `sqlite3`**: the guarantee
//! holds in the file, not in the Rust code. On the other hand, no trigger
//! survives a `DROP TABLE`, a `PRAGMA writable_schema` or rewriting the file
//! with a hex editor. The protection is against mistakes and against an agent
//! that would want to erase its trace through the product's ordinary means —
//! not against an attacker who already has write access to the user's disk.

use rusqlite::Connection;

use crate::error::{Result, StoreError};

/// A migration, as recorded in `schema_version`.
#[derive(Debug, Clone, Copy)]
pub(crate) struct Migration {
    /// Number, strictly increasing and never reused.
    pub(crate) version: u32,
    /// Short name, written in `schema_version` to make the table readable.
    pub(crate) name: &'static str,
    /// The SQL batch. May contain several statements.
    pub(crate) sql: &'static str,
}

/// Migration tracking table. Created outside any migration: it is what says
/// which migrations remain to apply.
const SCHEMA_VERSION_TABLE: &str = "\
CREATE TABLE IF NOT EXISTS schema_version (
    version    INTEGER PRIMARY KEY NOT NULL,
    name       TEXT    NOT NULL,
    applied_at TEXT    NOT NULL
) STRICT;";

/// Migration 1 — the phase 0 schema.
const M0001_INITIAL: &str = "\
CREATE TABLE workspaces (
    id         TEXT PRIMARY KEY NOT NULL,
    name       TEXT NOT NULL,
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL
) STRICT;

-- `params` is a JSON object of NON-secret values; `secret_ref` is a
-- reference to the system keychain, never the secret (SECURITY, I-03).
CREATE TABLE connections (
    id           TEXT PRIMARY KEY NOT NULL,
    workspace_id TEXT NOT NULL REFERENCES workspaces(id) ON DELETE CASCADE,
    name         TEXT NOT NULL,
    driver       TEXT NOT NULL,
    environment  TEXT NOT NULL,
    params       TEXT NOT NULL,
    secret_ref   TEXT,
    read_only    INTEGER NOT NULL,
    created_at   TEXT NOT NULL,
    updated_at   TEXT NOT NULL
) STRICT;

CREATE INDEX connections_by_workspace ON connections (workspace_id, name);

-- `connection_id` deliberately has NO foreign key: deleting a connection
-- does not erase what the user ran with it. The name is copied so that the
-- history stays readable after that deletion.
CREATE TABLE query_history (
    id              INTEGER PRIMARY KEY AUTOINCREMENT,
    ts              TEXT NOT NULL,
    connection_id   TEXT,
    connection_name TEXT,
    actor_kind      TEXT NOT NULL,
    actor_id        TEXT,
    language        TEXT NOT NULL,
    statement       TEXT NOT NULL,
    intent          TEXT NOT NULL,
    duration_ms     INTEGER,
    row_count       INTEGER,
    status          TEXT NOT NULL,
    error           TEXT
) STRICT;

CREATE INDEX query_history_by_ts ON query_history (ts DESC);
CREATE INDEX query_history_by_connection ON query_history (connection_id, ts DESC);

-- APPEND-ONLY. No foreign key either: the audit trail survives the deletion
-- of the connection, the workspace and the agent it implicates.
-- AUTOINCREMENT rather than the bare rowid: a reused identifier would let one
-- row impersonate another in an audit trail.
CREATE TABLE audit_journal (
    id               INTEGER PRIMARY KEY AUTOINCREMENT,
    ts               TEXT NOT NULL,
    command_id       TEXT,
    actor_kind       TEXT NOT NULL,
    actor_id         TEXT,
    agent_session_id TEXT,
    connection_id    TEXT,
    command_kind     TEXT NOT NULL,
    statement        TEXT,
    intent           TEXT NOT NULL,
    risk             TEXT NOT NULL,
    policy_decision  TEXT NOT NULL,
    decision_reason  TEXT,
    approved_by      TEXT,
    duration_ms      INTEGER,
    rows_affected    INTEGER,
    error            TEXT
) STRICT;

CREATE INDEX audit_journal_by_ts ON audit_journal (ts DESC);
CREATE INDEX audit_journal_by_connection ON audit_journal (connection_id, ts DESC);

CREATE TRIGGER audit_journal_forbid_update
BEFORE UPDATE ON audit_journal
BEGIN
    SELECT RAISE(ABORT, 'audit_journal is append-only: UPDATE is forbidden');
END;

CREATE TRIGGER audit_journal_forbid_delete
BEFORE DELETE ON audit_journal
BEGIN
    SELECT RAISE(ABORT, 'audit_journal is append-only: DELETE is forbidden');
END;

CREATE TABLE catalog_cache (
    connection_id  TEXT PRIMARY KEY NOT NULL REFERENCES connections(id) ON DELETE CASCADE,
    payload        TEXT NOT NULL,
    refreshed_at   TEXT NOT NULL,
    server_version TEXT
) STRICT;

CREATE TABLE documents (
    id            TEXT PRIMARY KEY NOT NULL,
    workspace_id  TEXT NOT NULL REFERENCES workspaces(id) ON DELETE CASCADE,
    title         TEXT NOT NULL,
    language      TEXT NOT NULL,
    content       TEXT NOT NULL,
    connection_id TEXT REFERENCES connections(id) ON DELETE SET NULL,
    created_at    TEXT NOT NULL,
    updated_at    TEXT NOT NULL
) STRICT;

CREATE INDEX documents_by_workspace ON documents (workspace_id, updated_at DESC);
";

/// Migration 2 — the error's class, next to its message.
///
/// The message alone is not enough: "timed out after 30 s" does not say the
/// server may have applied the write. Without this column, an interface that
/// wants to withhold a "rerun" button has no choice but to parse text — which
/// `.claude/rules/rust.md` forbids precisely because a message changes and a
/// caller that parsed it silently breaks (I-13).
///
/// `NULL` on earlier rows, and on any row that did not fail.
const M0002_HISTORY_ERROR_CLASS: &str = "\
ALTER TABLE query_history ADD COLUMN error_class TEXT;";

/// Migration 3 — the same error class, in the audit trail.
///
/// `query_history` has carried it since migration 2; its absence from
/// `audit_journal` would be the asymmetry the wrong way round. It is the table
/// read **after** an incident, the one that records the commands the history
/// ignores, and the one that cannot be corrected afterwards: it is
/// append-only.
///
/// `ALTER TABLE ... ADD COLUMN` is not an `UPDATE`: the tamper-proofing
/// trigger does not object, and no existing row is rewritten.
const M0003_JOURNAL_ERROR_CLASS: &str = "\
ALTER TABLE audit_journal ADD COLUMN error_class TEXT;";

const M0004_WORKSPACE_PREFERENCES: &str = "CREATE TABLE workspace_preferences (
    workspace_id TEXT PRIMARY KEY NOT NULL REFERENCES workspaces(id) ON DELETE CASCADE,
    revision INTEGER NOT NULL CHECK(revision > 0),
    payload TEXT NOT NULL CHECK(length(CAST(payload AS BLOB)) <= 4096),
    updated_at TEXT NOT NULL
) STRICT;";

const M0005_QUERY_LIBRARY: &str = "
ALTER TABLE documents ADD COLUMN revision INTEGER NOT NULL DEFAULT 0;
ALTER TABLE documents ADD COLUMN saved_revision INTEGER NOT NULL DEFAULT 0;
ALTER TABLE documents ADD COLUMN is_saved INTEGER NOT NULL DEFAULT 1;
ALTER TABLE documents ADD COLUMN is_open INTEGER NOT NULL DEFAULT 0;
ALTER TABLE documents ADD COLUMN is_deleted INTEGER NOT NULL DEFAULT 0;
ALTER TABLE documents ADD COLUMN saved_content TEXT;
ALTER TABLE documents ADD COLUMN saved_title TEXT;
UPDATE documents SET saved_content=content, saved_title=title;
ALTER TABLE query_history ADD COLUMN result_id TEXT;
CREATE INDEX documents_by_open ON documents(workspace_id, is_open, id);
";

/// What tells a clean shutdown from an abnormal one.
///
/// `closed_at` is only set by an ordinary close, **after** local writes were
/// flushed; `heartbeat_at` ages while an instance works. A session without a
/// closing whose heartbeat has aged is a crash; a session without a closing
/// but with a recent heartbeat is another instance, very much alive
/// ([ADR-0021](../../../docs/adr/0021-marqueur-d-arret.md)).
///
/// The pid is not there: checking it would require what the repository's
/// `unsafe` policy refuses, and a reused pid would make the test lie.
const M0006_APP_SESSIONS: &str = "CREATE TABLE app_sessions (
    id           TEXT PRIMARY KEY NOT NULL,
    workspace_id TEXT NOT NULL REFERENCES workspaces(id) ON DELETE CASCADE,
    started_at   TEXT NOT NULL,
    heartbeat_at TEXT NOT NULL,
    closed_at    TEXT
) STRICT;
CREATE INDEX app_sessions_open ON app_sessions(workspace_id, closed_at, heartbeat_at);
";

/// A model provider is declared **per machine**, and a text written by an
/// agent carries its origin.
///
/// `ai_providers` has **no** `workspace_id`, and that is the decision, not an
/// oversight: an Ollama listening on the machine serves every workspace, and
/// duplicating it per workspace would create as many places where its
/// configuration can diverge. What stays per connection is the privacy tier —
/// a shared provider does not make a shared tier
/// ([ADR-0023](../../../docs/adr/0023-fournisseurs-declares-et-provenance.md),
/// [ADR-0006](../../../docs/adr/0006-ai-privacy-tiers.md)).
///
/// Direct consequence: the table has no foreign key, so deleting a workspace
/// does not make the user's providers disappear.
///
/// No key is written there: `secret_ref` designates an entry of the system
/// keychain, as for `connections` (I-03). `reach` has **no** column: the
/// local/remote classification is recomputed at every opening, because a
/// stored value would be yesterday's DNS answer applied to today's send.
///
/// `documents.provenance` is a JSON bounded to 512 bytes, `NULL` by default.
/// `NULL` means "written by the user" — it is the case of every existing row,
/// and it is true. The `CHECK` takes the shape of
/// `workspace_preferences.payload`: without it, this metadata would become the
/// place where "just a bit of context" gets stored, that is, model prompts
/// and answers in the workspace file.
const M0007_AI_PROVIDERS: &str = "CREATE TABLE ai_providers (
    id         TEXT PRIMARY KEY NOT NULL,
    kind       TEXT NOT NULL,
    label      TEXT NOT NULL,
    base_url   TEXT NOT NULL,
    model      TEXT NOT NULL,
    secret_ref TEXT,
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL
) STRICT;
CREATE INDEX ai_providers_by_label ON ai_providers (label, id);
ALTER TABLE documents ADD COLUMN provenance TEXT
    CHECK(provenance IS NULL OR length(CAST(provenance AS BLOB)) <= 512);
";

/// Migration 8 — the privacy tier, where it belongs.
///
/// [ADR-0006](../../../docs/adr/0006-ai-privacy-tiers.md) attaches the tier to
/// the **connection**, and `ConnectionConfig` has carried it from the start.
/// It was written nowhere: every connection read back reverted to the default
/// value, which made `Local` unreachable from one session to the next — a
/// setting made on a customer database was lost on close, without a message.
///
/// `NULL` on earlier rows, and that is **accurate**: they were written by a
/// binary that did not know this setting, so the user never chose one.
/// Reading applies ADR-0006's default, `Metadata`. An **unreadable** value,
/// on the other hand, is not an absence: reading then falls back to the most
/// restrictive tier, `Local`, because a database for which it is no longer
/// known what it allowed must not get the benefit of the doubt.
const M0008_CONNECTION_PRIVACY: &str = "ALTER TABLE connections ADD COLUMN privacy_tier TEXT;";

/// Migration 9 — the declared external agents.
///
/// [ADR-0026](../../../docs/adr/0026-agents-externes-acp.md) adds a second
/// destination mode: a program already installed on the user's machine,
/// launched as a subprocess. A table separate from `ai_providers`, not added
/// columns: an agent has neither endpoint, nor model, nor **secret
/// reference**, and making them share would have produced a table half of
/// whose columns mean nothing depending on the row.
///
/// **No secret column, and that is the point.** An agent carries its own
/// authentication; Oxyn holds none. The only sure way not to leak a key is not
/// to have it ([I-03](../../../CLAUDE.md#i-03)).
///
/// No `workspace_id` either, for the same reason as `ai_providers`: an agent
/// installed on the machine serves every workspace.
///
/// `args` and `env` are bounded JSON — the same `CHECK` shape as
/// `documents.provenance`. Without the bound, a state file written by a third
/// party would make the opening allocate whatever it wants. `env` **must
/// not** carry a secret: what is there goes into a process environment,
/// visible in the process table on some systems.
///
/// No reach column, and this time not because it would be stale as for a
/// provider: an external agent's reach is **unknowable**, so there is nothing
/// to write.
const M0009_EXTERNAL_AGENTS: &str = "CREATE TABLE external_agents (
    id         TEXT PRIMARY KEY NOT NULL,
    label      TEXT NOT NULL,
    command    TEXT NOT NULL,
    args       TEXT NOT NULL
        CHECK(length(CAST(args AS BLOB)) <= 4096),
    env        TEXT NOT NULL
        CHECK(length(CAST(env AS BLOB)) <= 4096),
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL
) STRICT;
CREATE INDEX external_agents_by_label ON external_agents (label, id);
";

/// Migration 10 — the assistant's conversations, and their turns.
///
/// Two tables and not one: a conversation has an identity, a title and a
/// destination that do not change at each turn, and copying them onto each
/// transcript row would turn a rename into a rewrite of the whole thread.
///
/// # What has **no** foreign key, and what has one
///
/// `ai_conversation_turns.conversation_id` has one, with `ON DELETE CASCADE`,
/// and it is the mechanism that keeps pruning's promise: deleting a
/// conversation takes its turns **in the same transaction**, so there is no
/// state where half a transcript remains. A transcript truncated in the middle
/// is a transcript that lies.
///
/// `connection_id` and `destination_id` have none, exactly like
/// `query_history.connection_id`: deleting a connection or removing a provider
/// does not erase what the user asked with it. The connection name and the
/// destination label are copied so that the thread stays readable after that
/// deletion.
///
/// # The privacy tier is on the **turn**
///
/// Not on the conversation: a user can change a connection's tier in the
/// middle of a thread, and a tier stored at the top would then be wrong for
/// every earlier turn. An audit reading asks under which regime **that turn**
/// took place ([I-04](../../../CLAUDE.md#i-04),
/// [ADR-0006](../../../docs/adr/0006-ai-privacy-tiers.md)).
///
/// # What is bounded in the file, and why there
///
/// `text`, `reasoning` and `tool_calls` carry a size `CHECK` — the same shape
/// as `documents.provenance` and `workspace_preferences.payload`. The last two
/// are filled by a **provider**: without a bound, a runaway reasoning payload
/// would make the opening allocate whatever a third party wants. The bound
/// lives in the file and not only in the code, so it holds against `sqlite3`
/// as much as against Oxyn.
///
/// # What there is no column to write
///
/// Neither a tool call's arguments, nor a query's result, nor a bound value,
/// nor a key. `tool_calls` carries a **rendering**: tool name, statement,
/// outcome. It is the same method as `external_agents`, which has no secret
/// column: the sure way not to write a value is not to have a place to put it
/// ([I-03](../../../CLAUDE.md#i-03)).
const M0010_AI_CONVERSATIONS: &str = "CREATE TABLE ai_conversations (
    id                TEXT PRIMARY KEY NOT NULL,
    workspace_id      TEXT NOT NULL REFERENCES workspaces(id) ON DELETE CASCADE,
    connection_id     TEXT,
    connection_name   TEXT,
    destination_kind  TEXT NOT NULL,
    destination_id    TEXT,
    destination_label TEXT NOT NULL,
    model             TEXT,
    title             TEXT NOT NULL
        CHECK(length(CAST(title AS BLOB)) <= 512),
    created_at        TEXT NOT NULL,
    updated_at        TEXT NOT NULL
) STRICT;
CREATE INDEX ai_conversations_by_connection ON ai_conversations (connection_id, updated_at DESC);
CREATE INDEX ai_conversations_by_workspace ON ai_conversations (workspace_id, updated_at DESC);

CREATE TABLE ai_conversation_turns (
    conversation_id    TEXT NOT NULL REFERENCES ai_conversations(id) ON DELETE CASCADE,
    ordinal            INTEGER NOT NULL,
    ts                 TEXT NOT NULL,
    role               TEXT NOT NULL,
    privacy_tier       TEXT NOT NULL,
    agent_session_id   TEXT,
    text               TEXT NOT NULL
        CHECK(length(CAST(text AS BLOB)) <= 1048576),
    reasoning          TEXT
        CHECK(reasoning IS NULL OR length(CAST(reasoning AS BLOB)) <= 262144),
    tool_calls         TEXT
        CHECK(tool_calls IS NULL OR length(CAST(tool_calls AS BLOB)) <= 65536),
    stop_reason        TEXT,
    prompt_tokens      INTEGER,
    completion_tokens  INTEGER,
    cache_write_tokens INTEGER,
    cache_read_tokens  INTEGER,
    reasoning_tokens   INTEGER,
    PRIMARY KEY (conversation_id, ordinal)
) STRICT;
";

/// Migration 11 — the two bounds migration 10 had not put in the file: the
/// number of turns in a thread, and the size of `stop_reason`.
///
/// Without them, reading a thread back was only bounded by the code that
/// writes: a `StopReason::Other` the size of an SSE frame, or turns written by
/// `sqlite3` beyond the bound, and opening a thread allocated whatever a third
/// party wanted ([I-06](../../../CLAUDE.md#i-06)).
///
/// # Triggers, not a `CHECK`
///
/// SQLite does not add a `CHECK` to an existing table: it would have to be
/// rebuilt by copying its rows. And a row already on disk that exceeded the
/// new bound would then have only two outcomes, both wrong: failing the
/// migration — hence the opening of the local state —, or rewriting the
/// user's data during the copy.
///
/// A `BEFORE INSERT` / `BEFORE UPDATE` trigger bounds **writes**, including
/// those of a `sqlite3`, without touching what exists. What exists is read
/// back following `encoding.rs`'s rule: a `stop_reason` value that is too
/// large is never loaded and reads back as `Unspecified`, and surplus turns
/// are read in bounded pages like the others. It is the shape of
/// `audit_journal`'s tamper-proofing trigger, for the same reason: the
/// guarantee holds in the file, not in the Rust code.
///
/// The two numbers are those of `MAX_TURNS_PER_CONVERSATION` and
/// `MAX_STOP_REASON_BYTES`; a test checks they do not diverge.
const M0011_AI_CONVERSATION_BOUNDS: &str = "CREATE TRIGGER ai_conversation_turns_bounds_insert
BEFORE INSERT ON ai_conversation_turns
WHEN NEW.ordinal < 0 OR NEW.ordinal >= 512
  OR (NEW.stop_reason IS NOT NULL AND length(CAST(NEW.stop_reason AS BLOB)) > 256)
BEGIN
    SELECT RAISE(ABORT, 'ai_conversation_turns: ordinal or stop_reason exceeds its bound');
END;
CREATE TRIGGER ai_conversation_turns_bounds_update
BEFORE UPDATE OF ordinal, stop_reason ON ai_conversation_turns
WHEN NEW.ordinal < 0 OR NEW.ordinal >= 512
  OR (NEW.stop_reason IS NOT NULL AND length(CAST(NEW.stop_reason AS BLOB)) > 256)
BEGIN
    SELECT RAISE(ABORT, 'ai_conversation_turns: ordinal or stop_reason exceeds its bound');
END;
";

/// Migration 12 — the log of data egress to an AI recipient.
///
/// A separate table, not columns added to `audit_journal`: its columns —
/// `command_kind`, the `PolicyGate` triad, `statement`, `rows_affected` —
/// have another meaning, and repurposing them would make both questions
/// unreadable. The sample read stays logged there as the `PreviewRelation` it
/// is; `command_id` links the two.
///
/// # The audit journal's retention, that is, none
///
/// The same tamper-proofing triggers as `audit_journal`, the same absence of
/// foreign keys: the entry survives the deletion of the connection, the
/// provider and the conversation it names. An egress that could be erased
/// would answer "nothing left" to the only question it exists for.
///
/// # No value, and what the file refuses to guarantee it
///
/// There is no column for a value. What could let one through is `columns`, a
/// JSON list of **names**: the `CHECK` requires a bounded array, and the
/// trigger refuses any element that is not a string, that is empty or that
/// exceeds an identifier's length. A number, an object, a pasted row do not
/// fit. The bounds are those of `oxyn-store::egress`, and a test checks they
/// do not diverge.
const M0012_AI_EGRESS: &str = "CREATE TABLE ai_egress (
    id              INTEGER PRIMARY KEY AUTOINCREMENT,
    ts              TEXT NOT NULL,
    connection_id   TEXT NOT NULL,
    command_id      TEXT,
    source          TEXT NOT NULL
        CHECK(length(CAST(source AS BLOB)) BETWEEN 1 AND 1024),
    columns         TEXT NOT NULL
        -- json_array_length returns 0 for anything that is not an array:
        -- `BETWEEN 1` therefore requires a non-empty array, with no json_type clause.
        CHECK(json_valid(columns)
              AND json_array_length(columns) BETWEEN 1 AND 256
              AND length(CAST(columns AS BLOB)) <= 131072),
    row_count       INTEGER NOT NULL CHECK(row_count BETWEEN 0 AND 1000),
    recipient_id    TEXT NOT NULL CHECK(length(CAST(recipient_id AS BLOB)) <= 64),
    model           TEXT CHECK(model IS NULL OR length(CAST(model AS BLOB)) <= 128),
    reach           TEXT NOT NULL CHECK(length(CAST(reach AS BLOB)) <= 16),
    conversation_id TEXT,
    node            INTEGER CHECK(node IS NULL OR node >= 0)
) STRICT;
CREATE INDEX ai_egress_by_connection ON ai_egress (connection_id, id DESC);

CREATE TRIGGER ai_egress_column_names_only
BEFORE INSERT ON ai_egress
WHEN EXISTS (SELECT 1 FROM json_each(NEW.columns)
              WHERE type <> 'text' OR length(CAST(value AS BLOB)) NOT BETWEEN 1 AND 256)
BEGIN
    SELECT RAISE(ABORT, 'ai_egress.columns holds column names only');
END;

CREATE TRIGGER ai_egress_forbid_update
BEFORE UPDATE ON ai_egress
BEGIN
    SELECT RAISE(ABORT, 'ai_egress is append-only: UPDATE is forbidden');
END;

CREATE TRIGGER ai_egress_forbid_delete
BEFORE DELETE ON ai_egress
BEGIN
    SELECT RAISE(ABORT, 'ai_egress is append-only: DELETE is forbidden');
END;
";

/// Migration 13 — a conversation's tree, and the exchange that received a
/// sample.
///
/// Migration 10 stored a **linear** transcript. The panel holds a **tree**: a
/// regeneration or an edit creates a sibling version, and the user navigates
/// between them. `ai_conversation_nodes` carries an exchange — its question,
/// its recipient, its outcome — and existing turns attach to it through
/// `node`.
///
/// # What the file makes impossible, rather than checking it
///
/// * **A cycle.** `parent < node`: a parent is always older than its child,
///   and the identifier is assigned by the store on insert. No chain of
///   parents can loop back on itself, `sqlite3` included.
/// * **A parent from another conversation.** The foreign key is composite,
///   `(conversation_id, parent)`: the parent is looked up **in the same
///   conversation**, and nowhere else.
/// * **Changing the structure after the fact.** A trigger refuses updating
///   `conversation_id`, `node` or `parent`: a version that changed parent
///   would rewrite the history that was shown.
///
/// # The exchange that received a sample only keeps its question
///
/// The user's decision: an exchange for which a sample of rows was sent keeps
/// its question, its **counters** — rows and columns, never the names — and
/// its outcome. Never the answer, the reasoning, a tool call or an error
/// message: the answer can quote the sample's values, and the workspace file
/// is one of I-03's six channels.
///
/// The rule holds in the file, not in the caller:
/// * a trigger refuses any turn attached to a withheld exchange, on insert as
///   on update;
/// * setting the marker **erases** the turns already written for that
///   exchange;
/// * the marker cannot be removed.
///
/// The erasure relies on `secure_delete`, which the store sets on opening:
/// without it, SQLite leaves the deleted text in its free pages, and the file
/// would still contain it. A `sqlite3` launched without this setting can set
/// the marker without overwriting the bytes — the "unreadable on disk"
/// guarantee holds for what Oxyn writes.
///
/// # Older rows
///
/// Migration 10's turns have `node = NULL` and stay readable through
/// `transcript_page`. A conversation without nodes simply has no branch.
const M0013_AI_CONVERSATION_TREE: &str = "CREATE TABLE ai_conversation_nodes (
    conversation_id   TEXT NOT NULL REFERENCES ai_conversations(id) ON DELETE CASCADE,
    node              INTEGER NOT NULL CHECK(node BETWEEN 0 AND 255),
    parent            INTEGER CHECK(parent IS NULL OR (parent >= 0 AND parent < node)),
    created_at        TEXT NOT NULL,
    privacy_tier      TEXT NOT NULL CHECK(length(CAST(privacy_tier AS BLOB)) <= 16),
    question          TEXT NOT NULL CHECK(length(CAST(question AS BLOB)) <= 1048576),
    destination_kind  TEXT NOT NULL CHECK(length(CAST(destination_kind AS BLOB)) <= 16),
    destination_id    TEXT CHECK(destination_id IS NULL OR length(CAST(destination_id AS BLOB)) <= 64),
    destination_label TEXT NOT NULL CHECK(length(CAST(destination_label AS BLOB)) <= 128),
    model             TEXT CHECK(model IS NULL OR length(CAST(model AS BLOB)) <= 128),
    sample_withheld   INTEGER NOT NULL DEFAULT 0 CHECK(sample_withheld IN (0, 1)),
    sample_rows       INTEGER CHECK(sample_rows IS NULL OR sample_rows BETWEEN 0 AND 1000),
    sample_columns    INTEGER CHECK(sample_columns IS NULL OR sample_columns BETWEEN 0 AND 256),
    outcome           TEXT CHECK(outcome IS NULL OR length(CAST(outcome AS BLOB)) <= 16),
    outcome_detail    TEXT CHECK(outcome_detail IS NULL OR length(CAST(outcome_detail AS BLOB)) <= 32),
    retryable         INTEGER CHECK(retryable IS NULL OR retryable IN (0, 1)),
    CHECK(sample_withheld = 1 OR (sample_rows IS NULL AND sample_columns IS NULL)),
    CHECK(outcome IS NOT NULL OR (outcome_detail IS NULL AND retryable IS NULL)),
    PRIMARY KEY (conversation_id, node),
    FOREIGN KEY (conversation_id, parent)
        REFERENCES ai_conversation_nodes(conversation_id, node)
) STRICT;

ALTER TABLE ai_conversation_turns ADD COLUMN node INTEGER CHECK(node IS NULL OR node >= 0);
CREATE INDEX ai_conversation_turns_by_node ON ai_conversation_turns (conversation_id, node, ordinal);
ALTER TABLE ai_conversations ADD COLUMN selected_node INTEGER;

CREATE TRIGGER ai_conversation_nodes_structure_is_final
BEFORE UPDATE OF conversation_id, node, parent ON ai_conversation_nodes
BEGIN
    SELECT RAISE(ABORT, 'ai_conversation_nodes: a node never changes place');
END;

CREATE TRIGGER ai_conversation_nodes_withholding_is_final
BEFORE UPDATE OF sample_withheld ON ai_conversation_nodes
WHEN OLD.sample_withheld = 1 AND NEW.sample_withheld = 0
BEGIN
    SELECT RAISE(ABORT, 'ai_conversation_nodes: a withheld sample stays withheld');
END;

CREATE TRIGGER ai_conversation_nodes_withholding_erases_answers
AFTER UPDATE OF sample_withheld ON ai_conversation_nodes
WHEN NEW.sample_withheld = 1
BEGIN
    DELETE FROM ai_conversation_turns
     WHERE conversation_id = NEW.conversation_id AND node = NEW.node;
END;

CREATE TRIGGER ai_conversation_turns_node_insert
BEFORE INSERT ON ai_conversation_turns
WHEN NEW.node IS NOT NULL AND NOT EXISTS (
    SELECT 1 FROM ai_conversation_nodes
     WHERE conversation_id = NEW.conversation_id AND node = NEW.node AND sample_withheld = 0)
BEGIN
    SELECT RAISE(ABORT, 'ai_conversation_turns: unknown node, or a node whose sample is withheld');
END;

CREATE TRIGGER ai_conversation_turns_node_update
BEFORE UPDATE ON ai_conversation_turns
WHEN NEW.node IS NOT NULL AND NOT EXISTS (
    SELECT 1 FROM ai_conversation_nodes
     WHERE conversation_id = NEW.conversation_id AND node = NEW.node AND sample_withheld = 0)
BEGIN
    SELECT RAISE(ABORT, 'ai_conversation_turns: unknown node, or a node whose sample is withheld');
END;

CREATE TRIGGER ai_conversations_selected_node_insert
BEFORE INSERT ON ai_conversations
WHEN NEW.selected_node IS NOT NULL
BEGIN
    SELECT RAISE(ABORT, 'ai_conversations: a new conversation has no node to select');
END;

CREATE TRIGGER ai_conversations_selected_node_update
BEFORE UPDATE OF selected_node ON ai_conversations
WHEN NEW.selected_node IS NOT NULL AND NOT EXISTS (
    SELECT 1 FROM ai_conversation_nodes
     WHERE conversation_id = NEW.id AND node = NEW.selected_node)
BEGIN
    SELECT RAISE(ABORT, 'ai_conversations: the selected node belongs to another conversation');
END;
";

/// Migration 14 — what a question named with an `@`.
///
/// A conversation read back shows its questions as they were asked, chips
/// included. The list is a **JSON** array — readable without Oxyn
/// ([I-11](../../../CLAUDE.md#i-11)) — that the file checks and bounds; `NULL`
/// for a question without a mention, and for any earlier row: it reads back
/// without a chip. Never a row value: a kind, a name, an address.
const M0014_AI_EXCHANGE_MENTIONS: &str =
    "ALTER TABLE ai_conversation_nodes ADD COLUMN mentions TEXT
    CHECK(mentions IS NULL
          OR (json_valid(mentions) AND json_type(mentions) = 'array'
              AND length(CAST(mentions AS BLOB)) <= 32768));
";

/// Migration 15 — the persisted catalog cache leaves the file.
///
/// Migration 1's `catalog_cache` table never had a caller: the catalog lives
/// in memory in `oxyn-exec`, is read back from the server at each connection,
/// and Oxyn offers no offline browsing
/// ([ARCHITECTURE §6](../../../docs/ARCHITECTURE.md#6-the-catalog)). A table
/// without a writer makes a reader of the file believe it matters.
///
/// Nothing of the user's is lost: what it could have contained is a copy of
/// what the server returns, never work typed into Oxyn
/// ([I-11](../../../CLAUDE.md#i-11)). Going back means a migration that
/// recreates the table with migration 1's DDL, still readable above; a future
/// persistence will first go through an ADR.
///
/// `IF EXISTS`: a file whose table was already removed with `sqlite3` must
/// open, not fail the migration.
const M0015_DROP_CATALOG_CACHE: &str = "DROP TABLE IF EXISTS catalog_cache;";

/// Migration 16 — when the user checked on the server a write with an unknown outcome.
///
/// Without it, a single expired write brought back the recovery warning at
/// every launch, forever — and a warning that cannot be silenced stops being
/// read, including the day another write was really interrupted. An ISO 8601
/// text timestamp rather than a flag: readable by `sqlite3` without Oxyn
/// ([I-11](../../../CLAUDE.md#i-11)), and it says *when* the check was made.
/// `NULL` on any earlier row: nothing was checked.
const M0016_HISTORY_RECONCILED: &str = "ALTER TABLE query_history ADD COLUMN reconciled_at TEXT;";

/// Migration 17 — when a later launch announced an abnormal shutdown.
///
/// Without it, an abandoned session stayed so forever: a single crash showed
/// the recovery screen at every launch, clean shutdowns included. `closed_at`
/// stays `NULL` — recording a closing here would lie about how the launch
/// ended. A timestamp rather than a flag, for the same reason as migration 16:
/// it reads without Oxyn ([I-11](../../../CLAUDE.md#i-11)). `NULL` on any
/// earlier row: already abandoned sessions are announced one last time.
const M0017_APP_SESSIONS_REPORTED: &str = "ALTER TABLE app_sessions ADD COLUMN reported_at TEXT;";

/// Migration 18 — the window layout
/// ([ADR-0043](../../../docs/adr/0043-multi-fenetre.md)).
///
/// Columns rather than a JSON: a console belonging to a single window is a
/// constraint the file holds itself, `UNIQUE (document_id)`, and a write that
/// would violate it fails instead of producing two rival windows on the same
/// document. Readable with any SQLite client ([I-11](../../../CLAUDE.md#i-11)).
/// `app_session_id` names the last launch that wrote the row: a launch only
/// adopts those of a finished launch, not those of another live instance on
/// the same file. `object_location` has the shape it had in the preferences.
const M0018_WORKSPACE_WINDOWS: &str = "CREATE TABLE workspace_windows (
    id               TEXT PRIMARY KEY NOT NULL,
    workspace_id     TEXT NOT NULL REFERENCES workspaces(id) ON DELETE CASCADE,
    app_session_id   TEXT NOT NULL,
    ordinal          INTEGER NOT NULL,
    x                REAL,
    y                REAL,
    width            REAL NOT NULL,
    height           REAL NOT NULL,
    maximized        INTEGER NOT NULL,
    object_location  TEXT,
    active_document  TEXT REFERENCES documents(id) ON DELETE SET NULL,
    revision         INTEGER NOT NULL,
    updated_at       TEXT NOT NULL
) STRICT;
CREATE TABLE workspace_window_consoles (
    window_id    TEXT NOT NULL REFERENCES workspace_windows(id) ON DELETE CASCADE,
    document_id  TEXT NOT NULL REFERENCES documents(id) ON DELETE CASCADE,
    position     INTEGER NOT NULL,
    PRIMARY KEY (window_id, document_id),
    UNIQUE (document_id)
) STRICT;
CREATE INDEX workspace_windows_order ON workspace_windows(workspace_id, ordinal);
";

/// Migration 19 — the role that opened a conversation. Older rows keep NULL:
/// resolving that legacy value to the SQL agent belongs to the caller.
const M0019_CONVERSATION_AGENT: &str = "ALTER TABLE ai_conversations ADD COLUMN agent_id TEXT;";

/// Every migration, in application order.
pub(crate) const MIGRATIONS: &[Migration] = &[
    Migration {
        version: 1,
        name: "initial",
        sql: M0001_INITIAL,
    },
    Migration {
        version: 2,
        name: "history_error_class",
        sql: M0002_HISTORY_ERROR_CLASS,
    },
    Migration {
        version: 3,
        name: "journal_error_class",
        sql: M0003_JOURNAL_ERROR_CLASS,
    },
    Migration {
        version: 4,
        name: "workspace_preferences",
        sql: M0004_WORKSPACE_PREFERENCES,
    },
    Migration {
        version: 5,
        name: "query_library",
        sql: M0005_QUERY_LIBRARY,
    },
    Migration {
        version: 6,
        name: "app_sessions",
        sql: M0006_APP_SESSIONS,
    },
    Migration {
        version: 7,
        name: "ai_providers",
        sql: M0007_AI_PROVIDERS,
    },
    Migration {
        version: 8,
        name: "connection_privacy_tier",
        sql: M0008_CONNECTION_PRIVACY,
    },
    Migration {
        version: 9,
        name: "external_agents",
        sql: M0009_EXTERNAL_AGENTS,
    },
    Migration {
        version: 10,
        name: "ai_conversations",
        sql: M0010_AI_CONVERSATIONS,
    },
    Migration {
        version: 11,
        name: "ai_conversation_bounds",
        sql: M0011_AI_CONVERSATION_BOUNDS,
    },
    Migration {
        version: 12,
        name: "ai_egress",
        sql: M0012_AI_EGRESS,
    },
    Migration {
        version: 13,
        name: "ai_conversation_tree",
        sql: M0013_AI_CONVERSATION_TREE,
    },
    Migration {
        version: 14,
        name: "ai_exchange_mentions",
        sql: M0014_AI_EXCHANGE_MENTIONS,
    },
    Migration {
        version: 15,
        name: "drop_catalog_cache",
        sql: M0015_DROP_CATALOG_CACHE,
    },
    Migration {
        version: 16,
        name: "history_reconciled",
        sql: M0016_HISTORY_RECONCILED,
    },
    Migration {
        version: 17,
        name: "app_sessions_reported",
        sql: M0017_APP_SESSIONS_REPORTED,
    },
    Migration {
        version: 18,
        name: "workspace_windows",
        sql: M0018_WORKSPACE_WINDOWS,
    },
    Migration {
        version: 19,
        name: "conversation_agent",
        sql: M0019_CONVERSATION_AGENT,
    },
];

/// Schema version this binary can produce.
#[must_use]
pub fn latest_version() -> u32 {
    match MIGRATIONS.last() {
        Some(last) => last.version,
        // Unreachable as long as `MIGRATIONS` is not empty, but an `expect`
        // here would panic when the application opens.
        None => 0,
    }
}

/// Schema version currently recorded in the file.
///
/// Returns `0` on a new database.
///
/// # Errors
/// [`StoreError::Sqlite`] if the tracking table is unreadable.
pub fn current_version(conn: &Connection) -> Result<u32> {
    let tracking_present: i64 = conn.query_row(
        "SELECT COUNT(*) FROM sqlite_schema WHERE type = 'table' AND name = 'schema_version'",
        [],
        |row| row.get(0),
    )?;
    if tracking_present == 0 {
        return Ok(0);
    }

    // `MAX()` always returns a row, `NULL` on an empty table.
    let raw = conn.query_row("SELECT MAX(version) FROM schema_version", [], |row| {
        row.get::<_, Option<i64>>(0)
    })?;
    Ok(raw.map_or(0, |v| u32::try_from(v).unwrap_or(u32::MAX)))
}

/// Applies the missing migrations.
///
/// Idempotent: called on an already up-to-date database, it does nothing and
/// does not modify `schema_version`.
///
/// # Errors
/// * [`StoreError::SchemaTooRecent`] if the file comes from a later Oxyn
///   version — writing into a schema that is not understood is refused rather
///   than corrupting the audit trail;
/// * [`StoreError::Migration`] if an SQL batch is refused. The transaction is
///   then rolled back and the schema stays in its previous state.
pub fn migrate(conn: &mut Connection) -> Result<()> {
    conn.execute_batch(SCHEMA_VERSION_TABLE)?;

    let current = current_version(conn)?;
    let target = latest_version();
    if current > target {
        return Err(StoreError::SchemaTooRecent {
            found: current,
            supported: target,
        });
    }

    for migration in MIGRATIONS.iter().filter(|m| m.version > current) {
        let tx = conn.transaction()?;
        tx.execute_batch(migration.sql)
            .map_err(|source| StoreError::Migration {
                version: migration.version,
                name: migration.name,
                source,
            })?;
        tx.execute(
            "INSERT INTO schema_version (version, name, applied_at) VALUES (?1, ?2, ?3)",
            rusqlite::params![
                i64::from(migration.version),
                migration.name,
                chrono::Utc::now()
            ],
        )
        .map_err(|source| StoreError::Migration {
            version: migration.version,
            name: migration.name,
            source,
        })?;
        tx.commit()?;
        tracing::debug!(
            version = migration.version,
            name = migration.name,
            "applied local state migration"
        );
    }

    Ok(())
}

/// Builds a local state file **at exactly `version`**, for tests that need a
/// file written by an earlier build.
///
/// Applying only the migrations up to `version` is the one form that does not
/// rot: a test that instead opened a current file and undid the later
/// migrations by hand broke every time a migration was added — twice already.
///
/// # Panics
/// On any SQLite failure: this is test scaffolding, and a fixture that cannot
/// be built is a broken test, not a condition to handle.
#[cfg(test)]
pub(crate) fn file_at_version(path: &std::path::Path, version: u32) -> Connection {
    let mut conn = Connection::open(path).expect("fixture file opens");
    conn.pragma_update(None, "foreign_keys", true)
        .expect("foreign keys on the fixture");
    conn.execute_batch(SCHEMA_VERSION_TABLE)
        .expect("schema_version on the fixture");
    for migration in MIGRATIONS.iter().filter(|m| m.version <= version) {
        let tx = conn.transaction().expect("fixture transaction");
        tx.execute_batch(migration.sql)
            .expect("an earlier migration applies");
        tx.execute(
            "INSERT INTO schema_version (version, name, applied_at) VALUES (?1, ?2, ?3)",
            rusqlite::params![migration.version, migration.name, chrono::Utc::now()],
        )
        .expect("fixture version recorded");
        tx.commit().expect("fixture commit");
    }
    conn
}

#[cfg(test)]
mod tests;
