import { QueryClient, QueryClientProvider } from "@tanstack/react-query"
import { act, fireEvent, render, screen, waitFor } from "@testing-library/react"
import { afterEach, beforeAll, describe, expect, it, vi } from "vitest"

import { ActionOverlays } from "./action-overlays"
import { actionSources } from "@/lib/actions/context"
import { invoke } from "@/lib/actions/registry"

// ⌘P searches the catalog the workspace has loaded, through the bounded
// search of the sidebar, and nothing else: no tree read, no refresh, no
// query to the server (ADR-0041, point 7; I-06).

const ipc = vi.hoisted(() => ({
  searchCatalog: vi.fn(),
  catalogTree: vi.fn(),
  refreshCatalog: vi.fn(),
  previewRelation: vi.fn(),
}))

vi.mock("@/lib/ipc/metadata", () => ({ metadata: ipc }))

// cmdk measures its list and scrolls to the selection; jsdom does neither.
beforeAll(() => {
  vi.stubGlobal(
    "ResizeObserver",
    class {
      observe() {}
      unobserve() {}
      disconnect() {}
    }
  )
  Element.prototype.scrollIntoView = () => undefined
})

const HIT = {
  address: { catalog: null, namespace: "public", relation: "orders" },
  name: "orders",
  kind: "table",
  holdsRecords: true,
  matched: "relationName" as const,
  matchedFields: [],
}

function renderOverlays() {
  const client = new QueryClient({
    defaultOptions: { queries: { retry: false } },
  })
  return render(
    <QueryClientProvider client={client}>
      <ActionOverlays />
    </QueryClientProvider>
  )
}

afterEach(() => {
  vi.clearAllMocks()
  actionSources.setState(() => ({ sources: {}, owners: {} }))
})

describe("the quick open", () => {
  it("searches the loaded catalog only, and opens a hit as the tree does", async () => {
    ipc.searchCatalog.mockResolvedValue([HIT])
    const openObject = vi.fn()
    renderOverlays()
    act(() =>
      actionSources.setState((current) => ({
        ...current,
        sources: {
          ...current.sources,
          workspace: {
            state: {
              activeTab: null,
              tabCount: 0,
              consoleCount: 0,
              objectActive: false,
              hasAside: false,
              hasAssistant: false,
            },
            actions: {} as never,
          },
          catalog: { state: { connection: "c1" }, actions: { openObject } },
        },
      }))
    )

    act(() => void invoke("object.quickOpen", "keyboard"))
    const field = await screen.findByPlaceholderText("Open a table, a view…")
    expect(screen.getAllByText(/already loaded in the catalog/).length).toBe(2)
    expect(ipc.searchCatalog).not.toHaveBeenCalled()

    fireEvent.change(field, { target: { value: "ord" } })
    await waitFor(() =>
      expect(ipc.searchCatalog).toHaveBeenCalledWith("c1", "ord")
    )
    fireEvent.click(await screen.findByText("orders"))

    expect(openObject).toHaveBeenCalledWith({
      address: HIT.address,
      name: "orders",
      kind: "table",
      holdsRecords: true,
    })
    expect(ipc.catalogTree).not.toHaveBeenCalled()
    expect(ipc.refreshCatalog).not.toHaveBeenCalled()
    expect(ipc.previewRelation).not.toHaveBeenCalled()
  })
})
