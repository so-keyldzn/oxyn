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
type MockedPreviewState =
  | { status: "initial" }
  | {
      status: "error"
      message: string
      retryable: boolean
      missingModule?: string | null
    }
const previewState = vi.hoisted(() => {
  const holder: { current: MockedPreviewState } = {
    current: { status: "initial" },
  }
  return holder
})
type MockedDetailLoad =
  | { status: "idle" }
  | { status: "error"; message: string; missingModule?: string | null }
const detailLoad = vi.hoisted(() => {
  const holder: { current: MockedDetailLoad } = {
    current: { status: "idle" },
  }
  return holder
})

vi.mock("@/features/metadata/use-preview", () => ({
  usePreview: (options: { visible: boolean }) => {
    previewVisible(options.visible)
    return {
      state: previewState.current,
      status: previewState.current.status === "error" ? "failed" : "initial",
      applied: PLAIN_SHAPE,
      reshaped: false,
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
      detail: detailLoad.current,
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
  virtualTable: null,
  shadowOf: null,
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
        active
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
  previewState.current = { status: "initial" }
  detailLoad.current = { status: "idle" }
})

describe("a virtual table whose module is not loaded", () => {
  const vec: CatalogNode = {
    ...node,
    address: { catalog: null, namespace: "main", relation: "chunks_vec" },
    name: "chunks_vec",
    virtualTable: {
      module: "vec0",
      available: false,
      shadows: ["chunks_vec_info", "chunks_vec_rowids"],
    },
  }
  const message = "driver `sqlite` (permanent error): no such module: vec0"

  it("explains the failed read and links to where its data is", () => {
    previewState.current = {
      status: "error",
      message,
      retryable: false,
      missingModule: "vec0",
    }
    const onOpenRelated = vi.fn()
    renderView({ node: vec, onOpenRelated })

    expect(
      screen.getByText(/provided by the SQLite extension/).textContent
    ).toContain("vec0, which Oxyn does not load")
    // sqlite-vec is opt-in per connection: the way to it is said.
    expect(
      screen.getByText(/enable sqlite-vec for this connection/)
    ).toBeTruthy()
    expect(screen.queryByText("The statement failed")).toBeNull()
    // No claim of a previous shape: this was the first read.
    expect(screen.queryByText(/previous shape/)).toBeNull()

    fireEvent.click(screen.getByRole("button", { name: "chunks_vec_rowids" }))
    expect(onOpenRelated).toHaveBeenCalledWith({
      catalog: null,
      namespace: "main",
      relation: "chunks_vec_rowids",
    })
  })

  it("shows any other failure as it came", () => {
    previewState.current = {
      status: "error",
      message: "database is locked",
      retryable: true,
    }
    renderView({
      node: {
        ...vec,
        virtualTable: { module: "fts5", available: true, shadows: [] },
      },
    })
    expect(screen.getByText("The statement failed")).toBeTruthy()
    expect(screen.queryByText(/provided by the SQLite extension/)).toBeNull()
  })

  it("shows an invalid predicate's own error, even on a table marked unavailable", () => {
    // The predicate was refused before sending: nothing names a module, and
    // the explanation would hide the typo to fix.
    previewState.current = {
      status: "error",
      message:
        "query rejected: the preview predicate is not a condition Oxyn can read",
      retryable: false,
      missingModule: null,
    }
    renderView({ node: vec })
    expect(screen.getByText("The statement failed")).toBeTruthy()
    expect(screen.getByText(/not a condition Oxyn can read/)).toBeTruthy()
    expect(screen.queryByText(/provided by the SQLite extension/)).toBeNull()
  })

  it("explains the Structure tab the same way", () => {
    detailLoad.current = {
      status: "error",
      message,
      missingModule: "vec0",
    }
    renderView({ node: vec, initialTab: "structure" })
    expect(screen.getByText(/provided by the SQLite extension/)).toBeTruthy()
    expect(screen.queryByText(message)).toBeNull()
  })

  it("keeps the generic wording for another module", () => {
    previewState.current = {
      status: "error",
      message: "no such module: spellfix1",
      retryable: false,
      missingModule: "spellfix1",
    }
    renderView({
      node: {
        ...vec,
        virtualTable: { module: "spellfix1", available: false, shadows: [] },
      },
    })
    expect(
      screen.getByText(/provided by the SQLite extension/).textContent
    ).toContain("spellfix1")
    expect(screen.queryByText(/enable sqlite-vec/)).toBeNull()
  })

  it("offers no read that would fail the same way", () => {
    // Refresh, Apply and Sort would each read again, and fail again until the
    // connection is reopened with the module.
    const reading = {
      ...open,
      capabilities: ["SQL", "PREVIEW_FILTER", "PREVIEW_SORT"],
    }
    previewState.current = {
      status: "error",
      message,
      retryable: false,
      missingModule: "vec0",
    }
    renderView({ open: reading, node: vec })
    expect(screen.queryByRole("button", { name: /Refresh data/ })).toBeNull()
    expect(screen.queryByLabelText("Preview filter predicate")).toBeNull()
    expect(screen.queryByRole("button", { name: /Sort/ })).toBeNull()

    // Any other failure keeps them: fixing the predicate is the way out.
    cleanup()
    previewState.current = {
      status: "error",
      message: "query rejected: not a condition",
      retryable: false,
      missingModule: null,
    }
    renderView({ open: reading, node: vec })
    expect(screen.getByRole("button", { name: /Refresh data/ })).toBeTruthy()
    expect(screen.getByLabelText("Preview filter predicate")).toBeTruthy()
  })

  it("says a view reads such a table, without calling it one", () => {
    previewState.current = {
      status: "error",
      message,
      retryable: false,
      missingModule: "vec0",
    }
    renderView({
      node: {
        ...node,
        address: { catalog: null, namespace: "main", relation: "recent" },
        name: "recent",
        kind: "view",
      },
    })
    expect(
      screen.getByText(/reads a virtual table whose module/).textContent
    ).toContain("This view reads a virtual table whose module vec0")
    expect(screen.queryByText(/provided by the SQLite extension/)).toBeNull()
    expect(screen.queryByText(/stored in|No table storing/)).toBeNull()
    expect(screen.getByText(/enable sqlite-vec/)).toBeTruthy()
  })
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

describe("a hidden object tab", () => {
  it("never calls its preview visible, whatever its sub-view", () => {
    // Behind another tab, or in a hidden workspace: its Data sub-view is
    // still mounted, and a write must not read it (ADR-0022).
    const { rerender } = renderView({ active: false })
    expect(previewVisible).toHaveBeenCalled()
    expect(previewVisible).not.toHaveBeenCalledWith(true)

    rerender(
      <QueryClientProvider client={new QueryClient()}>
        <ObjectView
          open={open}
          node={node}
          active
          definitionWidth={424}
          onDefinitionWidthChange={() => undefined}
        />
      </QueryClientProvider>
    )
    expect(previewVisible).toHaveBeenLastCalledWith(true)
  })
})
