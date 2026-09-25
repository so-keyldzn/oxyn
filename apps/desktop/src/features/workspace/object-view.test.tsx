import { QueryClient, QueryClientProvider } from "@tanstack/react-query"
import { cleanup, fireEvent, render, screen } from "@testing-library/react"
import { afterEach, describe, expect, it, vi } from "vitest"

import { ObjectView } from "@/features/workspace/object-view"
import { PLAIN_SHAPE } from "@/lib/ipc/metadata"
import type { CatalogNode, OpenConnection } from "@/lib/ipc/types"

// What a restored object tab must not do on its own: read rows (the preview
// is `visible`) or metadata (`ensure`). Both are watched at their hook, the
// last step before the backend.
const previewVisible = vi.hoisted(() => vi.fn())
const ensure = vi.hoisted(() => vi.fn())

vi.mock("@/features/metadata/use-preview", () => ({
  usePreview: (options: { visible: boolean }) => {
    previewVisible(options.visible)
    return {
      state: { status: "initial" },
      status: "idle",
      applied: PLAIN_SHAPE,
      running: false,
      cancelling: false,
      startedAt: null,
      approvalRefused: false,
      pagination: null,
      refresh: vi.fn(),
      applyPredicate: vi.fn(),
      applySort: vi.fn(),
      page: vi.fn(),
      cancel: vi.fn(),
    }
  },
}))

// The definition panel beside the tabs measures itself; jsdom cannot.
vi.stubGlobal(
  "ResizeObserver",
  class {
    observe() {}
    unobserve() {}
    disconnect() {}
  }
)

// The wide layout: jsdom has no `matchMedia`, and Relations is a tab there.
vi.mock("@/features/workspace/use-compact", () => ({
  useCompact: () => false,
}))

vi.mock("@/features/metadata/use-relation-facets", () => ({
  useRelationFacets: () => ({
    facets: null,
    error: null,
    loads: {
      detail: { status: "idle" },
      constraints: { status: "idle" },
      incomingKeys: { status: "idle" },
      definition: { status: "idle" },
    },
    refresh: vi.fn(),
    cancel: vi.fn(),
    ensure,
  }),
}))

const open: OpenConnection = {
  connection: "018f0000-0000-7000-8000-000000000001",
  session: "018f0000-0000-7000-8000-000000000002",
  name: "billing replica",
  driver: "postgresql",
  environment: "production",
  readOnly: false,
  privacyTier: "metadata",
  capabilities: ["SQL", "INCOMING_FOREIGN_KEYS"],
  console: {
    session: "018f0000-0000-7000-8000-000000000003",
    capabilities: [],
    readOnly: false,
    transactionState: "unknown",
  },
}

const node: CatalogNode = {
  address: { catalog: null, namespace: "billing", relation: "invoices" },
  name: "invoices",
  kind: "table",
  holdsRecords: true,
  system: false,
  comment: null,
  loaded: false,
  stale: false,
  children: [],
}

function renderView(props: Partial<React.ComponentProps<typeof ObjectView>>) {
  const client = new QueryClient({
    defaultOptions: { queries: { retry: false } },
  })
  return render(
    <QueryClientProvider client={client}>
      <ObjectView
        open={open}
        node={node}
        definitionWidth={424}
        onDefinitionWidthChange={() => undefined}
        {...props}
      />
    </QueryClientProvider>
  )
}

afterEach(() => {
  cleanup()
  vi.clearAllMocks()
})

describe("a restored object tab", () => {
  it("reads nothing, and moves no saved place, until the user asks", () => {
    const onPlaceChange = vi.fn()
    renderView({ restored: "data", onPlaceChange })

    expect(screen.getByText("Restored from your last session")).toBeTruthy()
    expect(previewVisible).not.toHaveBeenCalledWith(true)
    expect(ensure).not.toHaveBeenCalled()
    expect(onPlaceChange).not.toHaveBeenCalled()

    fireEvent.click(screen.getByRole("button", { name: "Read it now" }))
    expect(previewVisible).toHaveBeenLastCalledWith(true)
    expect(onPlaceChange).toHaveBeenLastCalledWith("data")
    expect(screen.queryByText("Restored from your last session")).toBeNull()
  })

  it("opens on its saved sub-view and loads it only once chosen again", () => {
    const onPlaceChange = vi.fn()
    renderView({ restored: "incomingRelations", onPlaceChange })

    expect(
      screen
        .getByRole("tab", { name: "Relations" })
        .getAttribute("aria-selected")
    ).toBe("true")
    expect(ensure).not.toHaveBeenCalled()

    fireEvent.click(screen.getByRole("button", { name: "Read it now" }))
    expect(ensure).toHaveBeenCalledWith("incomingKeys")
    expect(onPlaceChange).toHaveBeenLastCalledWith("incomingRelations")
  })

  it("an object opened by the user reads at once", () => {
    renderView({})
    expect(previewVisible).toHaveBeenCalledWith(true)
    expect(screen.queryByText("Restored from your last session")).toBeNull()
  })
})
