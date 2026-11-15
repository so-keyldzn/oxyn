import { StrictMode } from "react"
import { QueryClient, QueryClientProvider } from "@tanstack/react-query"
import {
  act,
  cleanup,
  fireEvent,
  render,
  screen,
  waitFor,
} from "@testing-library/react"
import { afterEach, beforeEach, expect, it, vi } from "vitest"
import { SchemaMapTab } from "./schema-map-tab"
import { mapFixture } from "@/components/oxyn/schema-map-fixtures"
import type { SchemaMapView } from "@/components/oxyn/schema-map-view"
import type { SchemaMapLoaded, SchemaMapRequest } from "@/lib/ipc/schema-map"
import type { CatalogNode, OpenConnection } from "@/lib/ipc/types"

const ipc = vi.hoisted(() => ({
  next: 0,
  nodes: [] as Array<CatalogNode>,
  cancel: vi.fn(async (_id: string) => undefined),
  sample: vi.fn(),
  reads: [] as Array<{
    id: string
    request: SchemaMapRequest
    resolve: (map: SchemaMapLoaded) => void
    reject: (error: Error) => void
  }>,
}))
vi.mock("@/lib/ipc/client", () => ({
  BackendError: class extends Error {
    retryable = false
  },
  newCommandId: () => `map-${++ipc.next}`,
  backend: { cancel: ipc.cancel },
}))
vi.mock("@/lib/ipc/schema-map", () => ({
  schemaMaps: {
    load: (id: string, request: SchemaMapRequest) =>
      new Promise<SchemaMapLoaded>((resolve, reject) => {
        ipc.reads.push({ id, request, resolve, reject })
      }),
  },
}))
vi.mock("@/lib/ipc/metadata", () => ({
  metadata: {
    catalogTree: async () => ipc.nodes,
    refreshRelationFacet: ipc.sample,
  },
}))
vi.mock("@/features/metadata/refresh-signals", () => ({
  useRefreshSignal: () => undefined,
}))
vi.mock("@/components/oxyn/schema-map-view", () => ({
  SchemaMapView: (props: React.ComponentProps<typeof SchemaMapView>) => (
    <>
      <button
        disabled={props.state.loading}
        onClick={() => props.onLoad(false)}
      >
        Load map
      </button>
      <button disabled={props.state.loading} onClick={() => props.onLoad(true)}>
        Refresh map
      </button>
      <button onClick={props.onCancel}>Cancel map</button>
      <button
        onClick={() =>
          props.onScope(
            props.scopes.find((choice) => choice.scope.catalog === "other")
              ?.key ?? ""
          )
        }
      >
        Other scope
      </button>
      <p>{props.state.error?.message}</p>
      <p>{props.state.data?.projection.source.session}</p>
      <p>{props.state.loading ? "Reading" : "Idle"}</p>
    </>
  ),
}))
const owner = (session: string) =>
  ({
    connection: "fixture-source",
    session,
    name: "Disposable source",
    capabilities: ["TABLES"],
    driver: "sqlite",
  }) as OpenConnection
function fixture(session = "fixture-owner") {
  const map = mapFixture()
  map.projection.source.session = session
  return map
}
function show(session = "fixture-owner", active = true) {
  const client = new QueryClient({
    defaultOptions: { queries: { retry: false } },
  })
  const frame = (visible: boolean) => (
    <StrictMode>
      <QueryClientProvider client={client}>
        <SchemaMapTab
          open={owner(session)}
          active={visible}
          initialScope={null}
          onOpenObject={vi.fn()}
        />
      </QueryClientProvider>
    </StrictMode>
  )
  const rendered = render(frame(active))
  return {
    ...rendered,
    setActive: (visible: boolean) => rendered.rerender(frame(visible)),
  }
}
beforeEach(() => {
  ipc.reads.length = 0
  ipc.nodes = []
  ipc.next = 0
  ipc.cancel.mockClear()
  ipc.sample.mockClear()
})
afterEach(cleanup)
it("opening a visible map loads one bounded demand, including StrictMode, without sampling", async () => {
  show()
  await waitFor(() => expect(ipc.reads).toHaveLength(1))
  expect(ipc.reads[0]?.request).toMatchObject({
    scope: { catalog: null, namespace: null },
    refresh: false,
    expand: null,
  })
  expect(ipc.sample).not.toHaveBeenCalled()
})
it("coalesces repeated clicks while loading and retains the exact original owner", async () => {
  show()
  fireEvent.click(screen.getByText("Load map"))
  fireEvent.click(screen.getByText("Load map"))
  expect(ipc.reads).toHaveLength(1)
  expect(ipc.reads[0]?.request).toMatchObject({
    connection: "fixture-source",
    session: "fixture-owner",
    expand: null,
    refresh: false,
  })
  await act(async () => {
    ipc.reads[0]?.resolve(fixture())
  })
  expect(screen.getByText("fixture-owner")).toBeTruthy()
  expect(ipc.sample).not.toHaveBeenCalled()
})
it("cancels only its own pending command when the tab closes and ignores a late answer", async () => {
  const first = show()
  fireEvent.click(screen.getByText("Load map"))
  const old = ipc.reads[0]
  first.unmount()
  expect(ipc.cancel).toHaveBeenCalledWith(old?.id)
  show("other-owner")
  fireEvent.click(screen.getByText("Load map"))
  await act(async () => {
    old?.resolve(fixture())
  })
  expect(screen.queryByText("fixture-owner")).toBeNull()
  await act(async () => {
    ipc.reads[1]?.resolve(fixture("other-owner"))
  })
  expect(screen.getByText("other-owner")).toBeTruthy()
})
it("refuses a response for another session and preserves the previous map after a failed refresh", async () => {
  show()
  fireEvent.click(screen.getByText("Load map"))
  await act(async () => {
    ipc.reads[0]?.resolve(fixture("wrong-owner"))
  })
  expect(screen.getByText("The map answered for another owner.")).toBeTruthy()
  expect(screen.queryByText("wrong-owner")).toBeNull()
  fireEvent.click(screen.getByText("Load map"))
  await act(async () => {
    ipc.reads[1]?.resolve(fixture())
  })
  fireEvent.click(screen.getByText("Refresh map"))
  await act(async () => {
    ipc.reads[2]?.reject(new Error("original file removed"))
  })
  expect(screen.getByText("original file removed")).toBeTruthy()
  expect(screen.getByText("fixture-owner")).toBeTruthy()
})

it("switching scope cancels the previous demand and rejects its late response", async () => {
  ipc.nodes = ["first", "other"].map(
    (name) =>
      ({
        kind: "catalog",
        name,
        address: { catalog: name, namespace: null, relation: null },
        children: [],
        holdsRecords: false,
        system: false,
        comment: null,
        loaded: false,
        stale: false,
        virtualTable: null,
        shadowOf: null,
      }) satisfies CatalogNode
  )
  show()
  await waitFor(() => expect(ipc.reads).toHaveLength(1))
  const old = ipc.reads[0]
  fireEvent.click(screen.getByText("Other scope"))
  await waitFor(() => expect(ipc.reads).toHaveLength(2))
  expect(ipc.cancel).toHaveBeenCalledWith(old?.id)
  expect(ipc.reads[1]?.request.scope.catalog).toBe("other")
  await act(async () => {
    old?.resolve(fixture("late-owner"))
  })
  expect(screen.queryByText("late-owner")).toBeNull()
  await act(async () => {
    ipc.reads[1]?.resolve(fixture())
  })
  expect(screen.getByText("fixture-owner")).toBeTruthy()
  expect(ipc.sample).not.toHaveBeenCalled()
})

it("a hidden retained map does not demand metadata and cancels only its own visible read", async () => {
  const view = show("fixture-owner", false)
  await act(async () => {
    await Promise.resolve()
  })
  expect(ipc.reads).toHaveLength(0)
  view.setActive(true)
  await waitFor(() => expect(ipc.reads).toHaveLength(1))
  const first = ipc.reads[0]
  view.setActive(false)
  expect(ipc.cancel).toHaveBeenCalledWith(first?.id)
  await act(async () => {
    first?.resolve(fixture("hidden-late"))
  })
  expect(screen.queryByText("hidden-late")).toBeNull()
  view.setActive(true)
  await waitFor(() => expect(ipc.reads).toHaveLength(2))
  expect(ipc.sample).not.toHaveBeenCalled()
})
