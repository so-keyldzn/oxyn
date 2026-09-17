// What a session declares, turned into what the screen may say about it — the
// port of `crates/oxyn-ui/src/session_capabilities.rs`.
//
// An absent capability is announced; it is never emulated (ADR-0003). The list
// is returned whole, supported entries included: a list that only ever shows
// problems stops being read.

export interface SurfaceSupport {
  surface: string
  supported: boolean
  detail: string
}

const SURFACES: ReadonlyArray<{
  surface: string
  capability: string
  yes: string
  no: string
}> = [
  {
    surface: "Server cancellation",
    capability: "SERVER_SIDE_CANCEL",
    yes: "Cancelling reaches the server and frees the connection.",
    // Only what the missing flag proves: no cancellation goes to a server.
    // SQLite has no server, and its interrupt does stop the statement.
    no: "Cancelling is not sent to a server: it stops the read here.",
  },
  {
    surface: "Transactions",
    capability: "TRANSACTIONS",
    yes: "BEGIN, COMMIT and ROLLBACK are real on this session.",
    no: "This session has no transaction: rollback is offered only when supported.",
  },
  {
    surface: "Savepoints",
    capability: "SAVEPOINTS",
    yes: "Named savepoints can be set inside a transaction.",
    no: "No savepoint: a partial rollback is not available on this session.",
  },
  {
    surface: "EXPLAIN",
    capability: "EXPLAIN",
    yes: "A query plan can be read without executing the statement.",
    no: "This session gives no query plan.",
  },
  {
    surface: "EXPLAIN ANALYZE",
    capability: "EXPLAIN_ANALYZE",
    yes: "The plan is measured by executing the statement it analyses.",
    no: "No measured plan on this session.",
  },
  {
    surface: "Multiple statements",
    capability: "MULTIPLE_STATEMENTS",
    yes: "Several statements may be submitted together.",
    no: "One statement per submission; a batch is never split silently.",
  },
  {
    surface: "Affected rows",
    capability: "AFFECTED_ROWS",
    yes: "The row count reported by a write is reliable.",
    no: "The row count reported by a write is not reliable on this session.",
  },
  {
    surface: "Indexes",
    capability: "INDEXES",
    yes: "Indexes are read from the server.",
    no: "Indexes are not introspectable on this session.",
  },
  {
    surface: "Constraints",
    capability: "CONSTRAINTS",
    yes: "Declared by this session.",
    no: "Constraints are not introspectable on this session.",
  },
  {
    surface: "Foreign keys",
    capability: "FOREIGN_KEYS",
    yes: "Foreign keys are read from the server, never guessed from column names.",
    no: "Foreign keys are not introspectable on this session.",
  },
]

/** Every surface this version conditions on a capability, in reading order. */
export function surfaces(capabilities: ReadonlyArray<string>) {
  const declared = new Set(capabilities)
  return SURFACES.map(({ surface, capability, yes, no }): SurfaceSupport => ({
    surface,
    supported: declared.has(capability),
    detail: declared.has(capability) ? yes : no,
  }))
}

/** Whether the session itself refuses writes, whatever the connection says. */
export function isReadOnlySession(capabilities: ReadonlyArray<string>) {
  return capabilities.includes("READ_ONLY_SESSION")
}
