import { beforeEach, describe, expect, it } from "vitest"

import {
  applyEvent,
  catalogVersion,
  executionProgress,
  forget,
  forgetSession,
  recordTransactionState,
  transactionStates,
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

describe("transaction state", () => {
  beforeEach(() => {
    executionProgress.setState(() => ({}))
    transactionStates.setState(() => ({}))
  })

  it("keeps the last state each session reported, whoever ran on it", () => {
    recordTransactionState("s1", "idle")
    // Watched or not: the state belongs to the session, not to the command.
    applyEvent({
      command: "unwatched",
      type: "transactionState",
      session: "s1",
      state: "open",
    })
    expect(transactionStates.state).toEqual({ s1: "open" })
  })

  it("changes on the event, never on the run that sends BEGIN", () => {
    recordTransactionState("s1", "idle")
    watch("begin", "s1")
    applyEvent({ command: "begin", type: "schemaReady", result: "r" })
    expect(transactionStates.state.s1).toBe("idle")
  })

  it("turns an abandoned run's session unknown, never idle", () => {
    recordTransactionState("s1", "open")
    watch("abandoned", "s1")
    // The answer may arrive first and forget the command's progress.
    forget("abandoned")
    applyEvent({ command: "abandoned", type: "cancelled" })
    expect(transactionStates.state.s1).toBe("unknown")
  })

  it("keeps the reported state when the cancellation follows it", () => {
    recordTransactionState("s1", "idle")
    watch("stopped", "s1")
    applyEvent({
      command: "stopped",
      type: "transactionState",
      session: "s1",
      state: "open",
    })
    applyEvent({ command: "stopped", type: "cancelled" })
    expect(transactionStates.state.s1).toBe("open")
  })

  it("forgets a closed session, and the runs still waiting on it", () => {
    recordTransactionState("s1", "open")
    watch("refused", "s1")
    forgetSession("s1")
    expect(transactionStates.state).toEqual({})
    applyEvent({ command: "refused", type: "cancelled" })
    expect(transactionStates.state).toEqual({})
  })
})
