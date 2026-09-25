import { afterEach, describe, expect, it, vi } from "vitest"

import { registerDraftFlush, subscribeToShutdown } from "./draft-registry"
import { exitHold, releaseExit } from "@/features/recovery/exit-transactions"
import type { ShutdownSignal } from "@/lib/ipc/recovery"

const backend = vi.hoisted(() => ({
  signal: null as ((signal: ShutdownSignal) => void) | null,
  confirmed: 0,
  acknowledged: 0,
}))

vi.mock("@/lib/ipc/recovery", () => ({
  recovery: {
    subscribeShutdown: (onSignal: (signal: ShutdownSignal) => void) => {
      backend.signal = onSignal
      return Promise.resolve()
    },
    shutdownFlushed: () => {
      backend.confirmed += 1
      return Promise.resolve()
    },
    shutdownAcknowledged: () => {
      backend.acknowledged += 1
      return Promise.resolve()
    },
  },
}))

/** Sends `flushDrafts`, then lets every settled flush answer the backend. */
async function shutdown() {
  backend.signal?.({ type: "flushDrafts" })
  // Not a delay: a macrotask runs after every microtask already queued.
  await new Promise((resolve) => setTimeout(resolve, 0))
}

describe("the shutdown flush", () => {
  const unregister: Array<() => void> = []
  afterEach(() => {
    unregister.splice(0).forEach((done) => done())
    backend.confirmed = 0
    backend.acknowledged = 0
    releaseExit()
  })

  it("is confirmed once every draft is in the store", async () => {
    subscribeToShutdown()
    unregister.push(registerDraftFlush("a", () => Promise.resolve(true)))
    unregister.push(registerDraftFlush("b", () => Promise.resolve(true)))
    await shutdown()
    expect(backend.confirmed).toBe(1)
  })

  it("is not confirmed when a draft did not reach the store", async () => {
    subscribeToShutdown()
    unregister.push(registerDraftFlush("a", () => Promise.resolve(true)))
    unregister.push(registerDraftFlush("b", () => Promise.resolve(false)))
    await shutdown()
    expect(backend.confirmed).toBe(0)
  })

  it("is not confirmed when a flush threw", async () => {
    subscribeToShutdown()
    unregister.push(registerDraftFlush("a", () => Promise.resolve(true)))
    unregister.push(
      registerDraftFlush("b", () => Promise.reject(new Error("gone")))
    )
    await shutdown()
    expect(backend.confirmed).toBe(0)
  })

  it("acknowledges the transactions to resolve, then holds them", () => {
    subscribeToShutdown()
    const transactions = [
      {
        session: "s1",
        connection: "c1",
        connectionName: "billing",
        environment: "production" as const,
        state: "open" as const,
      },
    ]
    backend.signal?.({ type: "resolveTransactions", transactions })
    expect(backend.acknowledged).toBe(1)
    expect(backend.confirmed).toBe(0)
    expect(exitHold.state?.transactions).toEqual(transactions)

    backend.signal?.({ type: "exitCancelled" })
    expect(exitHold.state).toBeNull()
  })
})
