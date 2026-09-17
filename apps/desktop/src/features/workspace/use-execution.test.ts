import { describe, expect, it } from "vitest"

import { stateFromOutcome } from "./use-execution"

describe("stateFromOutcome", () => {
  it("never presents a truncated result as complete", () => {
    const { state } = stateFromOutcome({
      type: "executed",
      result: "r",
      columns: [{ name: "id", dataType: "Int64", nullable: false }],
      rows: 2_000_000,
      elapsedMs: 900,
      complete: false,
      cancelled: false,
      truncated: true,
    })
    expect(state).toMatchObject({
      status: "populated",
      truncated: true,
      complete: false,
    })
  })

  it("holds a production write for review instead of running it", () => {
    const { state, approval } = stateFromOutcome({
      type: "needsApproval",
      command: "c",
      reason: "Writes on production need a review.",
      preview: {
        statement: "DELETE FROM orders",
        connection: "prod",
        estimatedRows: null,
      },
    })
    expect(state.status).toBe("initial")
    expect(approval?.preview?.connection).toBe("prod")
  })

  it("shows a denial as a non-retryable error", () => {
    const { state } = stateFromOutcome({
      type: "denied",
      reason: "Read-only connection",
    })
    expect(state).toEqual({
      status: "error",
      message: "Read-only connection",
      retryable: false,
    })
  })

  it("distinguishes a cancellation from an empty result and from an error", () => {
    expect(stateFromOutcome({ type: "cancelled" }).state.status).toBe("empty")
    expect(stateFromOutcome({ type: "cancelled" }).summary.status).toBe(
      "cancelled"
    )
  })
})
