import * as React from "react"
import { QueryClient, QueryClientProvider } from "@tanstack/react-query"
import { act, cleanup, renderHook, waitFor } from "@testing-library/react"
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest"

import { useCompletionCatalog } from "./use-completion-catalog"
import type { CompletionCatalog } from "@/lib/ipc/completion"
import type { RefreshSignal } from "@/lib/ipc/metadata"

const channel = vi.hoisted(() => ({
  emit: (_signal: unknown) => {},
}))
const reads = vi.hoisted(() => ({
  count: 0,
  fail: false,
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
    catalog: (connection: string) => {
      reads.count += 1
      if (reads.fail) return Promise.reject(new Error("not connected"))
      return Promise.resolve({
        namespaces: [],
        resolution: { state: "unknown" },
        truncated: null,
        // Each read is told apart by the number of the read.
        seen: `${connection}#${reads.count}`,
      })
    },
  },
}))

const send = (signal: RefreshSignal) => act(() => channel.emit(signal))

function open(connection = "c1") {
  const client = new QueryClient()
  const hook = renderHook(() => useCompletionCatalog(connection), {
    wrapper: ({ children }: { children: React.ReactNode }) => (
      <QueryClientProvider client={client}>{children}</QueryClientProvider>
    ),
  })
  return hook
}

const seen = (catalog: CompletionCatalog | null) =>
  (catalog as unknown as { seen?: string } | null)?.seen

beforeEach(() => {
  reads.count = 0
  reads.fail = false
})
afterEach(() => {
  // A hook left mounted would answer the signals of the next test too.
  cleanup()
})

describe("the completion snapshot of a connection", () => {
  it("is read once when the editor opens", async () => {
    const hook = open()
    expect(hook.result.current).toBeNull()
    await waitFor(() => expect(seen(hook.result.current)).toBe("c1#1"))
    expect(reads.count).toBe(1)
  })

  it("is read again, once, after a burst of catalog loads", async () => {
    const hook = open()
    await waitFor(() => expect(seen(hook.result.current)).toBe("c1#1"))
    for (let load = 0; load < 5; load += 1)
      send({ type: "catalogChanged", connection: "c1" })
    await waitFor(() => expect(seen(hook.result.current)).toBe("c1#2"))
    expect(reads.count).toBe(2)
  })

  it("is read again after a DDL and after missed signals", async () => {
    const hook = open()
    await waitFor(() => expect(seen(hook.result.current)).toBe("c1#1"))
    send({ type: "catalogInvalidated", connection: "c1" })
    await waitFor(() => expect(seen(hook.result.current)).toBe("c1#2"))
    send({ type: "lagged" })
    await waitFor(() => expect(seen(hook.result.current)).toBe("c1#3"))
  })

  it("ignores what concerns another connection, or something else", async () => {
    const hook = open()
    await waitFor(() => expect(seen(hook.result.current)).toBe("c1#1"))
    send({ type: "catalogChanged", connection: "c2" })
    send({ type: "rowsChanged", connection: "c1" })
    send({ type: "historyRecorded", connection: "c1" })
    // Longer than the coalescing delay: nothing was scheduled.
    await new Promise((resolve) => setTimeout(resolve, 300))
    expect(reads.count).toBe(1)
  })

  it("keeps the last snapshot while the next one is read", async () => {
    const hook = open()
    await waitFor(() => expect(seen(hook.result.current)).toBe("c1#1"))
    send({ type: "catalogChanged", connection: "c1" })
    expect(seen(hook.result.current)).toBe("c1#1")
    await waitFor(() => expect(seen(hook.result.current)).toBe("c1#2"))
  })

  it("is null while the connection has no cache, and reads once connected", async () => {
    reads.fail = true
    const hook = open()
    await waitFor(() => expect(reads.count).toBe(1))
    expect(hook.result.current).toBeNull()
    // The first load after the connection announces itself.
    reads.fail = false
    send({ type: "catalogChanged", connection: "c1" })
    await waitFor(() => expect(seen(hook.result.current)).toBe("c1#2"))
  })
})
