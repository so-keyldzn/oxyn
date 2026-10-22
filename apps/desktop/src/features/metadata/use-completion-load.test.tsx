import { act, cleanup, renderHook, waitFor } from "@testing-library/react"
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest"

import { useCompletionLoad } from "./use-completion-load"
import type { RefreshSignal } from "@/lib/ipc/metadata"

const channel = vi.hoisted(() => ({
  emit: (_signal: unknown) => {},
}))
const loads = vi.hoisted(() => ({
  calls: [] as Array<{ connection: string; names: unknown }>,
  answer: (): Promise<unknown> =>
    Promise.resolve({ attempted: 1, failed: 0, stopped: null }),
}))

vi.mock("@/lib/ipc/metadata", () => ({
  metadata: {
    subscribeRefreshSignals: (onSignal: (signal: unknown) => void) => {
      channel.emit = onSignal
      return Promise.resolve(null)
    },
  },
}))

vi.mock("@/lib/ipc/completion", () => ({
  completion: {
    load: (connection: string, names: unknown) => {
      loads.calls.push({ connection, names })
      return loads.answer()
    },
  },
}))

const send = (signal: RefreshSignal) => act(() => channel.emit(signal))

const users = [[{ text: "users", quoted: false }]]
const orders = [[{ text: "orders", quoted: false }]]

beforeEach(() => {
  loads.calls = []
  loads.answer = () =>
    Promise.resolve({ attempted: 1, failed: 0, stopped: null })
})
afterEach(() => {
  // A hook left mounted would answer the signals of the next test too.
  cleanup()
})

describe("the editor's request for what the catalog cache lacks", () => {
  it("sends the names of the statement to the host, with the connection", () => {
    const hook = renderHook(() => useCompletionLoad("c1", true))
    hook.result.current(users)
    expect(loads.calls).toEqual([{ connection: "c1", names: users }])
  })

  it("sends the same names once, and other names when they change", () => {
    const hook = renderHook(() => useCompletionLoad("c1", true))
    hook.result.current(users)
    hook.result.current(users)
    hook.result.current(orders)
    hook.result.current(users)
    expect(loads.calls.map((call) => call.names)).toEqual([
      users,
      orders,
      users,
    ])
  })

  it("sends a statement that names nothing too: the console wants its tables listed", () => {
    const hook = renderHook(() => useCompletionLoad("c1", true))
    hook.result.current([])
    expect(loads.calls).toEqual([{ connection: "c1", names: [] }])
  })

  it("sends the last names again after a DDL, with nothing new from the editor", () => {
    const hook = renderHook(() => useCompletionLoad("c1", true))
    hook.result.current(users)
    send({ type: "catalogInvalidated", connection: "c1" })
    expect(loads.calls.map((call) => call.names)).toEqual([users, users])
  })

  it("does not start a pass of its own before the editor spoke, nor for another connection", () => {
    renderHook(() => useCompletionLoad("c1", true))
    send({ type: "catalogInvalidated", connection: "c1" })
    expect(loads.calls).toEqual([])

    const hook = renderHook(() => useCompletionLoad("c1", true))
    hook.result.current(users)
    send({ type: "catalogInvalidated", connection: "c2" })
    send({ type: "catalogChanged", connection: "c1" })
    expect(loads.calls).toHaveLength(1)
  })

  it("continues with what a pass left when it ran out of time", async () => {
    loads.answer = () =>
      Promise.resolve({ attempted: 9, failed: 0, stopped: "deadline" })
    const hook = renderHook(() => useCompletionLoad("c1", true))
    hook.result.current(users)
    await waitFor(() => {
      hook.result.current(users)
      expect(loads.calls).toHaveLength(2)
    })
  })

  it("continues a pass another console of the connection replaced", async () => {
    loads.answer = () =>
      Promise.resolve({ attempted: 2, failed: 0, stopped: "cancelled" })
    const hook = renderHook(() => useCompletionLoad("c1", true))
    hook.result.current(users)
    await waitFor(() => {
      hook.result.current(users)
      expect(loads.calls).toHaveLength(2)
    })
  })

  it("does not take a pass back that its own newer names replaced", async () => {
    const answers: Array<(value: unknown) => void> = []
    loads.answer = () => new Promise((resolve) => answers.push(resolve))
    const hook = renderHook(() => useCompletionLoad("c1", true))
    hook.result.current(users)
    hook.result.current(orders)
    // The first pass ends « cancelled » because the second replaced it.
    await act(async () => {
      answers[0]?.({ attempted: 1, failed: 0, stopped: "cancelled" })
      await Promise.resolve()
    })
    hook.result.current(orders)
    expect(loads.calls.map((call) => call.names)).toEqual([users, orders])
  })

  it("tries again when the host could not be asked", async () => {
    loads.answer = () => Promise.reject(new Error("not connected"))
    const hook = renderHook(() => useCompletionLoad("c1", true))
    hook.result.current(users)
    await waitFor(() => {
      hook.result.current(users)
      expect(loads.calls.length).toBeGreaterThan(1)
    })
  })

  it("forgets what it sent for another connection", () => {
    const hook = renderHook(
      ({ connection }: { connection: string }) =>
        useCompletionLoad(connection, true),
      { initialProps: { connection: "c1" } }
    )
    hook.result.current(users)
    hook.rerender({ connection: "c2" })
    hook.result.current(users)
    expect(loads.calls.map((call) => call.connection)).toEqual(["c1", "c2"])
  })

  it("sends nothing from a console behind the active one, until it comes forward", () => {
    const hook = renderHook(
      ({ active }: { active: boolean }) => useCompletionLoad("c1", active),
      { initialProps: { active: false } }
    )
    hook.result.current(users)
    send({ type: "catalogInvalidated", connection: "c1" })
    expect(loads.calls).toEqual([])

    hook.rerender({ active: true })
    expect(loads.calls).toEqual([{ connection: "c1", names: users }])

    // And not again when it merely comes forward a second time.
    hook.rerender({ active: false })
    hook.rerender({ active: true })
    expect(loads.calls).toHaveLength(1)
  })

  it("reads again after a DDL that happened while it was behind", () => {
    const hook = renderHook(
      ({ active }: { active: boolean }) => useCompletionLoad("c1", active),
      { initialProps: { active: true } }
    )
    hook.result.current(users)
    hook.rerender({ active: false })
    send({ type: "catalogInvalidated", connection: "c1" })
    expect(loads.calls).toHaveLength(1)

    hook.rerender({ active: true })
    expect(loads.calls).toHaveLength(2)
  })
})
