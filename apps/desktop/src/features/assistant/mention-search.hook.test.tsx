import * as React from "react"
import { QueryClient, QueryClientProvider } from "@tanstack/react-query"
import { act, renderHook, waitFor } from "@testing-library/react"
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest"

import { useMentionSearch } from "./mention-search"
import type { CatalogNode } from "@/lib/ipc/types"

/** A slow backend: every answer takes this long. */
const LATENCY_MS = 300

const ipc = vi.hoisted(() => ({
  catalogTree: vi.fn(),
  searchCatalog: vi.fn(),
  listDocuments: vi.fn(),
  listMentionable: vi.fn(),
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
vi.mock("@/lib/ipc/ai", () => ({
  ai: { listMentionable: ipc.listMentionable },
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

const clients: Array<QueryClient> = []

const client = () => {
  const created = new QueryClient({
    defaultOptions: { queries: { retry: false } },
  })
  clients.push(created)
  return created
}

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
  ipc.listMentionable.mockImplementation(() => later(undefined))
})

// A test may end with an answer still on its way: it would land after the
// environment is torn down and notify a React root that no longer has a window.
afterEach(async () => {
  await waitFor(() =>
    expect(clients.every((each) => each.isFetching() === 0)).toBe(true)
  )
  clients.splice(0).forEach((each) => each.clear())
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
    // Opening the panel reads the cache only: the server waits for an `@`.
    expect(ipc.listMentionable).not.toHaveBeenCalled()
  })

  it("lists the tables of schemas never expanded, then offers them", async () => {
    const shared = client()
    // What a connection holds before anything is expanded: a schema, no table.
    shared.setQueryData(["catalog", "c5"], [node("public", null)])
    ipc.catalogTree.mockImplementation(() => later(TREE))
    const { result } = renderHook(() => useMentionSearch("c5"), {
      wrapper: withClient(shared),
    })
    act(() => result.current.onQuery(""))
    // Nothing to choose yet, and no verdict before the listing answers.
    expect(result.current.results).toMatchObject({
      status: "ready",
      choices: [],
      completing: true,
    })
    expect(ipc.listMentionable).toHaveBeenCalledWith("c5")
    await waitFor(() =>
      expect(labels(result.current.results)).toEqual(["orders", "customers"])
    )
    expect(result.current.results).toMatchObject({ completing: false })

    // Once per connection: the next `@` reads the cache it filled.
    act(() => result.current.onQuery(null))
    act(() => result.current.onQuery(""))
    expect(ipc.listMentionable).toHaveBeenCalledTimes(1)
  })

  it("searches again once the listing answered", async () => {
    const shared = client()
    shared.setQueryData(["catalog", "c6"], [node("public", null)])
    let listed = false
    ipc.listMentionable.mockImplementation(() =>
      later(undefined).then(() => {
        listed = true
      })
    )
    ipc.searchCatalog.mockImplementation(() =>
      later(
        listed
          ? [
              {
                address: {
                  catalog: null,
                  namespace: "public",
                  relation: "orders",
                },
                name: "orders",
                kind: "table",
                holdsRecords: true,
                matched: "relationName",
                matchedFields: [],
              },
            ]
          : []
      )
    )
    const { result } = renderHook(() => useMentionSearch("c6"), {
      wrapper: withClient(shared),
    })
    act(() => result.current.onQuery("ord"))
    // Both answer at once: the search asked before the listing landed must
    // not be the last word.
    await waitFor(() =>
      expect(labels(result.current.results)).toEqual(["orders"])
    )
    expect(result.current.results).toMatchObject({ completing: false })
  })

  it("offers what is loaded when the listing fails", async () => {
    const shared = client()
    shared.setQueryData(["catalog", "c7"], TREE)
    ipc.listMentionable.mockImplementation(() =>
      Promise.reject(new Error("server gone"))
    )
    const { result } = renderHook(() => useMentionSearch("c7"), {
      wrapper: withClient(shared),
    })
    act(() => result.current.onQuery(""))
    await waitFor(() =>
      expect(result.current.results).toMatchObject({
        status: "ready",
        completing: false,
      })
    )
    expect(labels(result.current.results)).toEqual(["orders", "customers"])
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
