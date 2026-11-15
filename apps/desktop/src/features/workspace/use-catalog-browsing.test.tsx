import { act, cleanup, renderHook, waitFor } from "@testing-library/react"
import { afterEach, beforeEach, expect, it, vi } from "vitest"
import { useCatalogBrowsing } from "./use-catalog-browsing"
import type { BrowsableConsole } from "./use-catalog-browsing"
import type { OpenConnection } from "@/lib/ipc/types"

const ipc = vi.hoisted(() => ({ cancel: vi.fn(async () => undefined) }))
vi.mock("@/features/metadata/read-requests", () => ({
  metadataReads: { cancelCatalog: ipc.cancel },
}))
const connection = {
  connection: "connection-a",
  session: "catalog-a",
  driver: "postgres",
  capabilities: ["TABLES"],
} as OpenConnection
const entries: Array<BrowsableConsole> = [
  {
    key: "console:a",
    title: "Draft A",
    session: {
      session: "session-a",
      capabilities: ["TABLES"],
      readOnly: false,
      transactionState: "open",
    },
  },
  {
    key: "console:b",
    title: "Draft B",
    session: {
      session: "session-b",
      capabilities: ["KEY_VALUE"],
      readOnly: true,
      transactionState: "unknown",
    },
  },
]
const compatible = (owner: OpenConnection) =>
  owner.capabilities.some((capability) =>
    ["TABLES", "KEY_VALUE"].includes(capability)
  )
beforeEach(() => ipc.cancel.mockClear())
afterEach(cleanup)
it("changes only the browsing owner and returns without changing a live console or retained owner snapshot", () => {
  const { result } = renderHook(() =>
    useCatalogBrowsing(connection, entries, compatible)
  )
  const before = structuredClone(entries)
  act(() => {
    expect(result.current.browse("console:a")).toBe(true)
  })
  const retainedOwner = result.current.owner
  expect(retainedOwner?.session).toBe("session-a")
  expect(result.current.name).toBe("Draft A")
  act(() => result.current.returnToConnection())
  expect(result.current.owner).toBeUndefined()
  expect(result.current.name).toBeNull()
  expect(retainedOwner?.session).toBe("session-a")
  expect(entries).toEqual(before)
  expect(ipc.cancel).toHaveBeenCalledWith("connection-a", "session-a")
})
it("re-resolves a menu callback against current entries after removal or window handoff", () => {
  const { result, rerender } = renderHook(
    ({ all }) => useCatalogBrowsing(connection, all, compatible),
    { initialProps: { all: entries } }
  )
  const capturedBrowse = result.current.browse
  const oldMenuAction = () => capturedBrowse("console:a")
  rerender({ all: entries.slice(1) })
  act(() => {
    expect(oldMenuAction()).toBe(false)
  })
  expect(result.current.owner).toBeUndefined()
  expect(ipc.cancel).not.toHaveBeenCalled()
})
it("quietly resets a retired source, cancelling that catalog read rather than another console", async () => {
  const { result, rerender } = renderHook(
    ({ all }) => useCatalogBrowsing(connection, all, compatible),
    { initialProps: { all: entries } }
  )
  act(() => {
    result.current.browse("console:a")
  })
  rerender({ all: entries.slice(1) })
  await waitFor(() =>
    expect(result.current.notice).toContain("browsing the connection again")
  )
  expect(result.current.owner).toBeUndefined()
  expect(ipc.cancel).toHaveBeenCalledWith("connection-a", "session-a")
  expect(ipc.cancel).not.toHaveBeenCalledWith("connection-a", "session-b")
})
it("gates on attached session capabilities, independent of protocol or transaction state", () => {
  const unavailable = [
    ...entries,
    { key: "offline", title: "Offline", session: null },
    {
      key: "silent",
      title: "No catalog",
      session: {
        session: "silent-session",
        capabilities: ["SQL"],
        readOnly: false,
        transactionState: "idle" as const,
      },
    },
  ]
  const { result } = renderHook(() =>
    useCatalogBrowsing(connection, unavailable, compatible)
  )
  act(() => {
    expect(result.current.browse("offline")).toBe(false)
    expect(result.current.browse("silent")).toBe(false)
    expect(result.current.browse("console:b")).toBe(true)
  })
  expect(result.current.owner?.session).toBe("session-b")
  expect(result.current.owner?.readOnly).toBe(true)
  expect(entries[1]?.session?.transactionState).toBe("unknown")
})

it("an already-open menu cannot select an unmounted workspace's console", () => {
  const { result, unmount } = renderHook(() =>
    useCatalogBrowsing(connection, entries, compatible)
  )
  const capturedBrowse = result.current.browse
  unmount()
  expect(capturedBrowse("console:a")).toBe(false)
  expect(ipc.cancel).not.toHaveBeenCalled()
})
