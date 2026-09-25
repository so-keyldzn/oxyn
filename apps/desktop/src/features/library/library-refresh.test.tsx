import * as React from "react"
import {
  QueryClient,
  QueryClientProvider,
  useQuery,
} from "@tanstack/react-query"
import { act, renderHook, waitFor } from "@testing-library/react"
import { describe, expect, it, vi } from "vitest"

import { LIBRARY_QUERY_KEY, useLibraryRefresh } from "./library-refresh"
import { useRefreshSignal } from "@/features/metadata/refresh-signals"
import type { RefreshSignal } from "@/lib/ipc/metadata"

const channel = vi.hoisted(() => ({
  emit: (_signal: unknown) => {},
}))

vi.mock("@/lib/ipc/metadata", () => ({
  metadata: {
    subscribeRefreshSignals: (onSignal: (signal: unknown) => void) => {
      channel.emit = onSignal
      return Promise.resolve(null)
    },
  },
}))

function withClient(client: QueryClient) {
  return function Wrapper({ children }: { children: React.ReactNode }) {
    return <QueryClientProvider client={client}>{children}</QueryClientProvider>
  }
}

/** An open library: one active query under the library key. */
function useOpenLibrary(reads: { count: number }) {
  useLibraryRefresh()
  return useQuery({
    queryKey: [...LIBRARY_QUERY_KEY, "history"],
    queryFn: () => ++reads.count,
  })
}

async function openLibrary() {
  const reads = { count: 0 }
  const client = new QueryClient({
    defaultOptions: { queries: { retry: false } },
  })
  const hook = renderHook(() => useOpenLibrary(reads), {
    wrapper: withClient(client),
  })
  await waitFor(() => expect(hook.result.current.data).toBe(1))
  return { reads, hook, client }
}

const send = (signal: RefreshSignal) => act(() => channel.emit(signal))

describe("the open library", () => {
  it("reads again when a run of any connection reaches the history", async () => {
    const { reads, hook } = await openLibrary()
    send({ type: "historyRecorded", connection: "another-connection" })
    await waitFor(() => expect(hook.result.current.data).toBe(2))
    expect(reads.count).toBe(2)
  })

  it("reads again after missed signals", async () => {
    const { hook } = await openLibrary()
    send({ type: "lagged" })
    await waitFor(() => expect(hook.result.current.data).toBe(2))
  })

  it("does not read again for a preview or a catalog change", async () => {
    const { reads, client } = await openLibrary()
    send({ type: "rowsChanged", connection: "c" })
    send({ type: "catalogInvalidated", connection: "c" })
    expect(
      client.getQueryState([...LIBRARY_QUERY_KEY, "history"])?.isInvalidated
    ).toBe(false)
    expect(reads.count).toBe(1)
  })

  it("is only marked stale while closed, and reads on its next opening", async () => {
    const { reads, hook, client } = await openLibrary()
    hook.unmount()
    const listening = renderHook(() => useLibraryRefresh(), {
      wrapper: withClient(client),
    })
    send({ type: "historyRecorded", connection: "c" })
    await waitFor(() =>
      expect(
        client.getQueryState([...LIBRARY_QUERY_KEY, "history"])?.isInvalidated
      ).toBe(true)
    )
    expect(reads.count).toBe(1)
    listening.unmount()
  })
})

describe("a view scoped to one connection", () => {
  it("hears its own connection and missed signals, not the others", () => {
    const heard: Array<RefreshSignal> = []
    const hook = renderHook(() =>
      useRefreshSignal("mine", (signal) => heard.push(signal))
    )
    send({ type: "rowsChanged", connection: "theirs" })
    send({ type: "rowsChanged", connection: "mine" })
    send({ type: "lagged" })
    expect(heard).toEqual([
      { type: "rowsChanged", connection: "mine" },
      { type: "lagged" },
    ])
    hook.unmount()
  })
})
