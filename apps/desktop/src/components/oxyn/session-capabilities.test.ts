import { describe, expect, it } from "vitest"

import { isReadOnlySession, surfaces } from "./session-capabilities"

// The real capability names of the two drivers, as `oxyn-core` spells them.
const postgres = [
  "SERVER_SIDE_CANCEL",
  "EXPLAIN",
  "EXPLAIN_ANALYZE",
  "AFFECTED_ROWS",
  "INDEXES",
  "CONSTRAINTS",
  "FOREIGN_KEYS",
]
const sqlite = [
  "TRANSACTIONS",
  "MULTIPLE_STATEMENTS",
  "EXPLAIN",
  "AFFECTED_ROWS",
  "INDEXES",
  "FOREIGN_KEYS",
]

const missing = (capabilities: Array<string>) =>
  surfaces(capabilities)
    .filter((surface) => !surface.supported)
    .map((surface) => surface.surface)

describe("surfaces", () => {
  it("differs between two real sessions", () => {
    expect(missing(postgres)).toContain("Transactions")
    expect(missing(sqlite)).not.toContain("Transactions")
    expect(missing(sqlite)).toContain("Server cancellation")
    expect(missing(postgres)).not.toContain("Server cancellation")
  })

  it("announces every surface of an empty session, with a reason", () => {
    const empty = surfaces([])
    expect(empty.length).toBeGreaterThan(0)
    for (const surface of empty) {
      expect(surface.supported).toBe(false)
      expect(surface.detail).not.toBe("")
    }
  })

  it("keeps the rollback wording", () => {
    const transactions = surfaces([]).find(
      (surface) => surface.surface === "Transactions"
    )
    expect(transactions?.detail).toContain("offered only when supported")
  })

  it("reads a read-only session from its capabilities", () => {
    expect(isReadOnlySession(["READ_ONLY_SESSION"])).toBe(true)
    expect(isReadOnlySession(postgres)).toBe(false)
  })
})
