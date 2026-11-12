import {
  QueryObserver,
  focusManager,
  onlineManager,
} from "@tanstack/react-query"
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest"

import { createQueryClient } from "./query-client"

const clients: Array<ReturnType<typeof createQueryClient>> = []
const subscriptions: Array<() => void> = []

function client() {
  const created = createQueryClient()
  created.mount()
  clients.push(created)
  return created
}

beforeEach(() => vi.useFakeTimers())
afterEach(() => {
  subscriptions.splice(0).forEach((unsubscribe) => unsubscribe())
  clients.splice(0).forEach((created) => {
    created.unmount()
    created.clear()
  })
  onlineManager.setOnline(true)
  focusManager.setFocused(undefined)
  vi.useRealTimers()
})

describe("local IPC query policy", () => {
  it("runs reads and writes offline, without replaying them on reconnect", async () => {
    onlineManager.setOnline(false)
    const created = client()
    const read = vi.fn(async () => "local result")
    const write = vi.fn(async () => "acknowledged")
    const observer = new QueryObserver(created, {
      queryKey: ["local-read"],
      queryFn: read,
    })
    subscriptions.push(observer.subscribe(() => {}))
    const mutation = created
      .getMutationCache()
      .build(created, { mutationFn: write })
    const done = mutation.execute(undefined)
    await vi.advanceTimersByTimeAsync(0)
    expect(observer.getCurrentResult()).toMatchObject({
      status: "success",
      fetchStatus: "idle",
      data: "local result",
    })
    expect(mutation.state).toMatchObject({ status: "success", isPaused: false })
    await expect(done).resolves.toBe("acknowledged")

    onlineManager.setOnline(true)
    focusManager.setFocused(false)
    focusManager.setFocused(true)
    await vi.advanceTimersByTimeAsync(30_000)
    expect(read).toHaveBeenCalledOnce()
    expect(write).toHaveBeenCalledOnce()
  })

  it("surfaces read failures and ambiguous write failures without retry", async () => {
    const created = client()
    const error = new Error("Outcome unknown")
    const read = vi.fn(async () => {
      throw error
    })
    const write = vi.fn(async () => {
      throw error
    })
    const query = created.fetchQuery({
      queryKey: ["failed-read"],
      queryFn: read,
    })
    const mutation = created
      .getMutationCache()
      .build(created, { mutationFn: write })
    const reading = expect(query).rejects.toBe(error)
    const writing = expect(mutation.execute(undefined)).rejects.toBe(error)
    await vi.advanceTimersByTimeAsync(30_000)
    await Promise.all([reading, writing])
    expect(read).toHaveBeenCalledOnce()
    expect(write).toHaveBeenCalledOnce()
    expect(created.getQueryState(["failed-read"])?.status).toBe("error")
    expect(mutation.state.status).toBe("error")
  })
})
