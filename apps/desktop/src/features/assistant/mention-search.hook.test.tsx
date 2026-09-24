import * as React from "react"
import { QueryClient, QueryClientProvider } from "@tanstack/react-query"
import { act, renderHook, waitFor } from "@testing-library/react"
import { beforeEach, describe, expect, it, vi } from "vitest"

import { useMentionSearch } from "./mention-search"
import type { CatalogNode } from "@/lib/ipc/types"

/** A slow backend: every answer takes this long. */
const LATENCY_MS = 300

const ipc = vi.hoisted(() => ({
  catalogTree: vi.fn(),
  searchCatalog: vi.fn(),
  listDocuments: vi.fn(),
}))

vi.mock("@/lib/ipc/metadata", () => ({
  metadata: {
    catalogTree: ipc.catalogTree,
    searchCatalog: ipc.searchCatalog,
  },
}))
vi.mock("@/lib/ipc/library", () => ({
  library: { listDocuments: ipc.listDocuments },
}))
vi.mock("@/lib/ipc/client", () => ({ newCommandId: () => "command" }))

function node(
  name: string,
  relation: string | null,
  children: Array<CatalogNode> = [],
  system = false
): CatalogNode {
  return {
    address: { catalog: null, namespace: "public", relation },
    name,
    kind: relation === null ? "schema" : "table",
    holdsRecords: relation !== null,
    system,
    comment: null,
    loaded: true,
    stale: false,
    children,
  }
}

const TREE = [
  node("pg_catalog", null, [node("pg_class", "pg_class")], true),
  node("public", null, [
    node("orders", "orders"),
    node("customers", "customers"),
  ]),
]

const later = <T,>(value: T) =>
  new Promise<T>((resolve) => setTimeout(() => resolve(value), LATENCY_MS))

function withClient(client: QueryClient) {
  return function Wrapper({ children }: { children: React.ReactNode }) {
    return <QueryClientProvider client={client}>{children}</QueryClientProvider>
  }
}

const client = () =>
  new QueryClient({ defaultOptions: { queries: { retry: false } } })

const labels = (
  results: ReturnType<typeof useMentionSearch>["results"]
): Array<string> | string =>
  results.status === "ready"
    ? results.choices.map((choice) => choice.label)
    : results.status

beforeEach(() => {
  vi.clearAllMocks()
  ipc.catalogTree.mockImplementation(() => later(TREE))
  ipc.searchCatalog.mockImplementation(() => later([]))
  ipc.listDocuments.mockImplementation(() => later({ entries: [] }))
})

describe("the first @", () => {
  it("shows the tree the sidebar already read, at the keystroke", () => {
    const shared = client()
    // What the sidebar left in the cache, under its own key.
    shared.setQueryData(["catalog", "c1"], TREE)
    const { result } = renderHook(() => useMentionSearch("c1"), {
      wrapper: withClient(shared),
    })

    const typed = performance.now()
    act(() => result.current.onQuery(""))
    // Same render: no wait on the slow backend, no debounce.
    expect(labels(result.current.results)).toEqual(["orders", "customers"])
    expect(performance.now() - typed).toBeLessThan(100)
    // Nothing asked again: the sidebar owns this tree's freshness.
    expect(ipc.catalogTree).not.toHaveBeenCalled()
  })

  it("warms the tree and the saved queries when the panel opens", async () => {
    renderHook(() => useMentionSearch("c2"), { wrapper: withClient(client()) })
    await waitFor(() => expect(ipc.catalogTree).toHaveBeenCalledWith("c2"))
    expect(ipc.listDocuments).toHaveBeenCalledWith(
      "command",
      expect.objectContaining({ search: "", savedOnly: true })
    )
  })

  it("waits on a cold cache without saying there is nothing", async () => {
    const { result } = renderHook(() => useMentionSearch("c3"), {
      wrapper: withClient(client()),
    })
    act(() => result.current.onQuery(""))
    expect(labels(result.current.results)).toBe("loading")
    await waitFor(() =>
      expect(labels(result.current.results)).toEqual(["orders", "customers"])
    )
  })

  it("keeps what it shows while the first typed letter is searched", async () => {
    const shared = client()
    shared.setQueryData(["catalog", "c4"], TREE)
    const { result } = renderHook(() => useMentionSearch("c4"), {
      wrapper: withClient(shared),
    })
    act(() => result.current.onQuery(""))
    act(() => result.current.onQuery("o"))
    // The tree's list stays, marked as completing: no blink, no empty verdict.
    expect(result.current.results).toMatchObject({
      status: "ready",
      completing: true,
    })
    expect(labels(result.current.results)).toEqual(["orders", "customers"])
    await waitFor(() =>
      expect(ipc.searchCatalog).toHaveBeenCalledWith("c4", "o")
    )
  })
})
