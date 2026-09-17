import { beforeEach, describe, expect, it } from "vitest"

import {
  applyEvent,
  catalogVersion,
  executionProgress,
  forget,
  watch,
} from "./events"

describe("execution progress", () => {
  beforeEach(() => {
    executionProgress.setState(() => ({}))
  })

  it("accumulates batches and knows the result before completion", () => {
    watch("c1")
    applyEvent({ command: "c1", type: "schemaReady", result: "r1" })
    applyEvent({ command: "c1", type: "batchReady", result: "r1", rows: 100 })
    applyEvent({ command: "c1", type: "batchReady", result: "r1", rows: 50 })
    expect(executionProgress.state.c1).toMatchObject({
      result: "r1",
      rows: 150,
      done: false,
    })
    applyEvent({
      command: "c1",
      type: "completed",
      result: "r1",
      rows: 150,
      elapsedMs: 12,
    })
    expect(executionProgress.state.c1?.done).toBe(true)
  })

  it("keeps the server's retry classification rather than guessing it", () => {
    watch("c2")
    applyEvent({
      command: "c2",
      type: "failed",
      error: "ERROR: canceling statement due to statement timeout",
      retryable: false,
    })
    expect(executionProgress.state.c2?.failed).toEqual({
      error: "ERROR: canceling statement due to statement timeout",
      retryable: false,
    })
  })

  it("ignores commands nobody watches, so the store does not grow", () => {
    applyEvent({
      command: "stranger",
      type: "batchReady",
      result: "r",
      rows: 1,
    })
    expect(executionProgress.state).toEqual({})
    watch("c3")
    forget("c3")
    expect(executionProgress.state).toEqual({})
  })

  it("bumps the catalog version when the backend says it changed", () => {
    const before = catalogVersion.state
    applyEvent({ command: "any", type: "catalogUpdated" })
    expect(catalogVersion.state).toBe(before + 1)
  })
})
