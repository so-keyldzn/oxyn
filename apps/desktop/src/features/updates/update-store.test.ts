import { beforeEach, describe, expect, it, vi } from "vitest"

import type { UpdateSnapshot } from "@/lib/ipc/updates"

const backend = vi.hoisted(() => ({
  push: null as null | ((snapshot: unknown) => void),
  answer: null as null | ((snapshot: unknown) => void),
}))

vi.mock("@/lib/ipc/updates", () => ({
  updates: {
    subscribe: (onSnapshot: (snapshot: unknown) => void) => {
      backend.push = onSnapshot
      return Promise.resolve(null)
    },
    state: () =>
      new Promise((resolve) => {
        backend.answer = resolve
      }),
  },
}))

const snapshot = (state: string) =>
  ({ state: { state }, automatic: true }) as unknown as UpdateSnapshot

async function freshStore() {
  vi.resetModules()
  return import("./update-store")
}

describe("the first snapshot of a window (ADR-0051)", () => {
  beforeEach(() => {
    backend.push = null
    backend.answer = null
  })

  it("keeps a channel message over an older answer to the read", async () => {
    const { subscribeToUpdates, updateStore } = await freshStore()
    subscribeToUpdates()
    backend.push?.(snapshot("ready"))
    backend.answer?.(snapshot("downloading"))
    await Promise.resolve()
    expect(updateStore.state.snapshot).toEqual(snapshot("ready"))
  })

  it("seeds from the read while the channel has said nothing", async () => {
    const { subscribeToUpdates, updateStore } = await freshStore()
    subscribeToUpdates()
    backend.answer?.(snapshot("idle"))
    await Promise.resolve()
    expect(updateStore.state.snapshot).toEqual(snapshot("idle"))
    backend.push?.(snapshot("checking"))
    expect(updateStore.state.snapshot).toEqual(snapshot("checking"))
  })
})
