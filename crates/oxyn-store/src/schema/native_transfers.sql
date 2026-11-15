-- Historical native outcomes survive connection removal; no credential or process log is stored.
CREATE TABLE native_transfer_jobs (
    id TEXT PRIMARY KEY NOT NULL,
    workspace_id TEXT NOT NULL REFERENCES workspaces(id) ON DELETE CASCADE,
    connection_id TEXT NOT NULL,
    owner_id TEXT NOT NULL,
    session_id TEXT NOT NULL,
    created_at TEXT NOT NULL,
    payload TEXT NOT NULL
);
CREATE INDEX native_transfer_jobs_source ON native_transfer_jobs(workspace_id, connection_id, created_at DESC, id DESC);
