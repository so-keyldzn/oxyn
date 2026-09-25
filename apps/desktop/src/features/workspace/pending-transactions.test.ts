import { describe, expect, it } from "vitest"

import { pendingTransaction } from "./pending-transactions"

describe("a hidden workspace's pending transaction", () => {
  it("is open when any console reported one open", () => {
    expect(
      pendingTransaction(
        [
          { session: "s1", transactions: true },
          { session: "s2", transactions: true },
        ],
        { s1: "unknown", s2: "open" }
      )
    ).toBe("open")
  })

  it("is unknown when a session that declares transactions never said", () => {
    // Never `idle` by default (ADR-0039 §5): an unanswered session may hold
    // one.
    expect(
      pendingTransaction([{ session: "s1", transactions: true }], {})
    ).toBe("unknown")
  })

  it("says nothing for idle sessions, or ones without transactions", () => {
    expect(
      pendingTransaction(
        [
          { session: "s1", transactions: true },
          { session: "s2", transactions: false },
        ],
        { s1: "idle", s2: "open" }
      )
    ).toBeNull()
  })
})
