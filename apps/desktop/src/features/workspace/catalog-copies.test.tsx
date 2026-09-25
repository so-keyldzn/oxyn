import type * as React from "react"
import { QueryClient, QueryClientProvider } from "@tanstack/react-query"
import { renderHook } from "@testing-library/react"
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest"

import { useCatalogCopies } from "./catalog-copies"
import type { RelationFacets } from "@/lib/ipc/metadata"
import type { CatalogNode, OpenConnection } from "@/lib/ipc/types"

const relationFacets = vi.hoisted(() => vi.fn())
const refreshRelationFacet = vi.hoisted(() => vi.fn())
const composeObjectSql = vi.hoisted(() => vi.fn())
const toastAdd = vi.hoisted(() => vi.fn())

vi.mock("@/lib/ipc/metadata", () => ({
  metadata: { relationFacets, refreshRelationFacet, composeObjectSql },
}))
vi.mock("@/lib/ipc/client", () => ({
  backend: { decide: vi.fn() },
  newCommandId: () => "command-1",
}))
vi.mock("@/components/ui/toast", () => ({ toast: { add: toastAdd } }))

const NEVER = { freshness: { state: "never" as const }, value: null }

function facets(detailRead: boolean): RelationFacets {
  return {
    address: { catalog: null, namespace: "public", relation: "orders" },
    kind: "table",
    holdsRecords: true,
    qualifiedName: '"public"."orders"',
    detail: detailRead
      ? {
          freshness: { state: "fetched", fetchedAt: "2026-09-25T10:00:00Z" },
          value: {
            name: "orders",
            kind: "table",
            comment: null,
            estimatedRows: null,
            sizeBytes: null,
            fields: [],
          },
        }
      : NEVER,
    indexes: null,
    foreignKeys: null,
    constraints: NEVER,
    incomingKeys: NEVER,
    definition: NEVER,
    uniqueKey: null,
  }
}

const open = {
  connection: "connection-1",
  session: "session-1",
} as OpenConnection

const orders: CatalogNode = {
  address: { catalog: null, namespace: "public", relation: "orders" },
  name: "orders",
  kind: "table",
  holdsRecords: true,
  system: false,
  comment: null,
  loaded: false,
  stale: false,
  children: [],
}

const INSERT = 'INSERT INTO "public"."orders" ("id") VALUES (?)'

class FakeClipboardItem {
  constructor(readonly items: Record<string, Promise<Blob>>) {}
}

let written: Array<Promise<string | undefined>>
let write: ReturnType<typeof vi.fn>

beforeEach(() => {
  written = []
  write = vi.fn(async (items: Array<FakeClipboardItem>) => {
    const blob = await items[0]?.items["text/plain"]
    written.push(Promise.resolve(blob?.text()))
  })
  Object.defineProperty(navigator, "clipboard", {
    value: { write, writeText: vi.fn() },
    configurable: true,
  })
  vi.stubGlobal("ClipboardItem", FakeClipboardItem)
  relationFacets.mockReset()
  refreshRelationFacet.mockReset()
  composeObjectSql.mockReset()
  toastAdd.mockReset()
})

afterEach(() => {
  Reflect.deleteProperty(navigator, "clipboard")
  vi.unstubAllGlobals()
})

function copies() {
  const client = new QueryClient({
    defaultOptions: { queries: { retry: false } },
  })
  const wrapper = ({ children }: { children: React.ReactNode }) => (
    <QueryClientProvider client={client}>{children}</QueryClientProvider>
  )
  return renderHook(() => useCatalogCopies(open), { wrapper }).result.current
}

describe("the catalog's copies", () => {
  it("asks the clipboard in the click, before the backend answers", () => {
    relationFacets.mockReturnValue(new Promise(() => undefined))
    copies().copyName(orders)
    expect(write).toHaveBeenCalledTimes(1)
  })

  it("reads the columns through the bus before an INSERT template", async () => {
    relationFacets
      .mockResolvedValueOnce(facets(false))
      .mockResolvedValue(facets(true))
    refreshRelationFacet.mockResolvedValue({ type: "catalogRefreshed" })
    composeObjectSql.mockResolvedValue(INSERT)

    copies().copyAs(orders, "insertTemplate")

    await vi.waitFor(() => expect(toastAdd).toHaveBeenCalled())
    expect(refreshRelationFacet).toHaveBeenCalledWith(
      "command-1",
      "connection-1",
      "session-1",
      orders.address,
      "detail"
    )
    expect(refreshRelationFacet.mock.invocationCallOrder[0]).toBeLessThan(
      composeObjectSql.mock.invocationCallOrder[0] ?? 0
    )
    await expect(written[0]).resolves.toBe(INSERT)
    expect(toastAdd).toHaveBeenCalledWith({
      title: "INSERT template copied",
      type: "success",
    })
  })

  it("does not read the structure again when it is loaded", async () => {
    relationFacets.mockResolvedValue(facets(true))
    composeObjectSql.mockResolvedValue(INSERT)

    copies().copyAs(orders, "insertTemplate")

    await vi.waitFor(() => expect(toastAdd).toHaveBeenCalled())
    expect(refreshRelationFacet).not.toHaveBeenCalled()
  })

  it("says a failed copy as a copy that failed", async () => {
    relationFacets.mockResolvedValue(facets(false))
    refreshRelationFacet.mockResolvedValue({
      type: "denied",
      reason: "The connection policy forbids reading this schema.",
    })

    copies().copyAs(orders, "insertTemplate")

    await vi.waitFor(() =>
      expect(toastAdd).toHaveBeenCalledWith({
        title: "INSERT template not copied",
        description: "The connection policy forbids reading this schema.",
        type: "error",
      })
    )
    expect(composeObjectSql).not.toHaveBeenCalled()
  })
})
