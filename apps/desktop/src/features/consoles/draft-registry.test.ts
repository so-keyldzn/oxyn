import { afterEach, describe, expect, it, vi } from "vitest"

import { registerDraftFlush, subscribeToShutdown } from "./draft-registry"

const backend = vi.hoisted(() => ({
  signal: null as (() => void) | null,
  confirmed: 0,
}))

vi.mock("@/lib/ipc/recovery", () => ({
  recovery: {
    subscribeShutdown: (onSignal: () => void) => {
      backend.signal = onSignal
      return Promise.resolve()
    },
    shutdownFlushed: () => {
      backend.confirmed += 1
      return Promise.resolve()
    },
  },
}))

/** Sends `flushDrafts`, then lets every settled flush answer the backend. */
async function shutdown() {
  backend.signal?.()
  // Not a delay: a macrotask runs after every microtask already queued.
  await new Promise((resolve) => setTimeout(resolve, 0))
}

describe("the shutdown flush", () => {
  const unregister: Array<() => void> = []
  afterEach(() => {
    unregister.splice(0).forEach((done) => done())
    backend.confirmed = 0
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
})
