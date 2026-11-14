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
import type { OpenConnection } from "@/lib/ipc/types"

const ipc = vi.hoisted(() => ({
  next: 0,
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
  metadata: { catalogTree: async () => [], refreshRelationFacet: ipc.sample },
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
function show(session = "fixture-owner") {
  const client = new QueryClient({
    defaultOptions: { queries: { retry: false } },
  })
  return render(
    <StrictMode>
      <QueryClientProvider client={client}>
        <SchemaMapTab
          open={owner(session)}
          active
          initialScope={null}
          onOpenObject={vi.fn()}
        />
      </QueryClientProvider>
    </StrictMode>
  )
}
beforeEach(() => {
  ipc.reads.length = 0
  ipc.next = 0
  ipc.cancel.mockClear()
  ipc.sample.mockClear()
})
afterEach(cleanup)
it("mounting, including StrictMode, does not load native metadata or sample rows", async () => {
  show()
  await waitFor(() => expect(screen.getByText("Idle")).toBeTruthy())
  expect(ipc.reads).toHaveLength(0)
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
