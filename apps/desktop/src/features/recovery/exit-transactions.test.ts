import { afterEach, describe, expect, it, vi } from "vitest"

import {
  commitBlocked,
  exitHold,
  holdExit,
  releaseExit,
  resolveExit,
  shownTransactions,
} from "./exit-transactions"
import type { ExitTransaction } from "@/lib/ipc/recovery"

const backend = vi.hoisted(() => ({
  sent: [] as Array<{ session: string; sql: string }>,
  /** Sessions whose statement fails, with the server's message. */
  failing: new Map<string, string>(),
  exits: 0,
  cancels: 0,
  closes: 0,
}))

vi.mock("@/lib/ipc/consoles", async () => {
  const { BackendError } = await import("@/lib/ipc/client")
  return {
    consoles: {
      run: (
        _command: string,
        _connection: string,
        session: string,
        run: { sql: string }
      ) => {
        backend.sent.push({ session, sql: run.sql })
        const failure = backend.failing.get(session)
        if (failure)
          return Promise.reject(
            new BackendError({ message: failure, retryable: false })
          )
        return Promise.resolve({
          type: "executed",
          result: "r",
          columns: [],
          rows: 0,
          elapsedMs: 1,
          complete: true,
          cancelled: false,
          truncated: false,
        })
      },
    },
  }
})

vi.mock("@/lib/ipc/results", () => ({
  results: { forgetResult: () => Promise.resolve() },
}))

vi.mock("@/lib/ipc/recovery", () => ({
  recovery: {
    requestExit: () => {
      backend.exits += 1
      return Promise.resolve()
    },
    cancelExit: () => {
      backend.cancels += 1
      return Promise.resolve()
    },
  },
}))

vi.mock("@/lib/ipc/windows", () => ({
  windows: {
    close: () => {
      backend.closes += 1
      return Promise.resolve()
    },
  },
}))

function transaction(session: string): ExitTransaction {
  return {
    session,
    connection: "c1",
    connectionName: "billing",
    environment: "production",
    state: "open",
  }
}

describe("resolving an exit held by transactions", () => {
  afterEach(() => {
    releaseExit()
    backend.sent = []
    backend.failing.clear()
    backend.exits = 0
    backend.cancels = 0
    backend.closes = 0
  })

  it("commits each session through the console path, then asks for the exit again", async () => {
    const listed = [transaction("s1"), transaction("s2")]
    holdExit(listed)
    await resolveExit("commit", listed)
    expect(backend.sent).toEqual([
      { session: "s1", sql: "COMMIT" },
      { session: "s2", sql: "COMMIT" },
    ])
    expect(backend.exits).toBe(1)
  })

  it("asks again for this window's close, not for the exit, when it held the close", async () => {
    const listed = [transaction("s1")]
    holdExit(listed, "window")
    await resolveExit("rollback", listed)
    expect(backend.sent).toEqual([{ session: "s1", sql: "ROLLBACK" }])
    expect(backend.closes).toBe(1)
    expect(backend.exits).toBe(0)
  })

  it("stops at a refused COMMIT, shows the server's message and never sends it again", async () => {
    const listed = [transaction("s1"), transaction("s2")]
    holdExit(listed)
    backend.failing.set("s1", "FOREIGN KEY constraint failed")
    await resolveExit("commit", listed)
    expect(backend.sent).toEqual([{ session: "s1", sql: "COMMIT" }])
    expect(exitHold.state?.error).toBe("FOREIGN KEY constraint failed")
    expect(backend.exits).toBe(0)

    // Commit again: the failed session is skipped, the other one committed.
    backend.failing.clear()
    await resolveExit("commit", listed)
    expect(backend.sent).toEqual([
      { session: "s1", sql: "COMMIT" },
      { session: "s2", sql: "COMMIT" },
    ])

    // A new listing from the backend keeps the memory of the failure.
    holdExit([transaction("s1")])
    const hold = exitHold.state
    if (!hold) throw new Error("the exit is held")
    const shown = shownTransactions(hold, {}, {})
    expect(commitBlocked(hold, shown)).toBe(true)
    await resolveExit("commit", shown)
    expect(backend.sent.filter((sent) => sent.session === "s1")).toHaveLength(1)

    // Rolling back stays possible.
    await resolveExit("rollback", shown)
    expect(backend.sent.at(-1)).toEqual({ session: "s1", sql: "ROLLBACK" })
  })

  it("does not ask for the exit once cancelled", async () => {
    const listed = [transaction("s1")]
    holdExit(listed)
    const resolving = resolveExit("rollback", listed)
    releaseExit()
    await resolving
    expect(backend.exits).toBe(0)
  })

  it("shows the listing's states until a statement runs, then the sessions' own", () => {
    holdExit([transaction("s1"), transaction("s2")])
    const hold = exitHold.state
    if (!hold) throw new Error("the exit is held")
    const labels = { s1: "orders.sql" }
    expect(
      shownTransactions(hold, labels, { s1: "idle" }).map((row) => row.console)
    ).toEqual(["orders.sql", "Console"])
    expect(
      shownTransactions({ ...hold, ran: true }, labels, { s1: "idle" }).map(
        (row) => row.session
      )
    ).toEqual(["s2"])
  })
})
