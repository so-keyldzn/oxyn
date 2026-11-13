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

import { CatalogSidebar } from "./catalog-sidebar"
import { metadataReads } from "@/features/metadata/read-requests"
import type { RefreshSignal } from "@/lib/ipc/metadata"
import type { CommandOutcome, OpenConnection } from "@/lib/ipc/types"

const ipc = vi.hoisted(() => ({
  next: 0,
  cancel: vi.fn(() => Promise.resolve()),
  emit: (_signal: RefreshSignal): void => undefined,
  reads: [] as Array<{
    id: string
    connection: string
    session: string
    resolve: (outcome: CommandOutcome) => void
    outcome: Promise<CommandOutcome>
  }>,
}))

vi.mock("@/lib/ipc/client", () => ({
  BackendError: class extends Error {
    retryable = false
  },
  backend: { cancel: ipc.cancel },
  newCommandId: () => `catalog-${++ipc.next}`,
}))
vi.mock("@/lib/ipc/metadata", () => ({
  metadata: {
    catalogTree: () => Promise.resolve([]),
    searchCatalog: () => Promise.resolve([]),
    refreshCatalog: (id: string, connection: string, session: string) => {
      let resolve!: (outcome: CommandOutcome) => void
      const outcome = new Promise<CommandOutcome>((reply) => {
        resolve = reply
      })
      ipc.reads.push({ id, connection, session, resolve, outcome })
      return outcome
    },
  },
}))
vi.mock("@/features/metadata/refresh-signals", () => ({
  useRefreshSignal: (
    _connection: string,
    listener: (signal: RefreshSignal) => void
  ) => {
    ipc.emit = listener
  },
}))
vi.mock("@/features/assistant/object-pin", () => ({
  usePinToQuestion: () => undefined,
}))
vi.mock("@/features/metadata/object-operation-flow", () => ({
  useObjectOperation: () => ({ start: vi.fn(), element: null }),
}))
vi.mock("./catalog-copies", () => ({
  useCatalogCopies: () => ({ copyName: vi.fn(), copyAs: vi.fn() }),
}))
vi.mock("@/components/ui/sidebar", () => ({
  Sidebar: ({ children }: { children: React.ReactNode }) => (
    <div>{children}</div>
  ),
  SidebarRail: () => null,
}))
vi.mock("@/components/oxyn/catalog-panel", () => ({
  CatalogPanel: ({
    refreshing,
    notice,
    onCancelRefresh,
  }: {
    refreshing: boolean
    notice: string | null
    onCancelRefresh: () => void
  }) => (
    <>
      <button disabled={!refreshing} onClick={onCancelRefresh}>
        Cancel catalog
      </button>
      <p>{notice}</p>
    </>
  ),
}))

function show(session = "console-a") {
  const client = new QueryClient({
    defaultOptions: { queries: { retry: false } },
  })
  const open = {
    connection: "connection",
    session,
    name: "temporary data",
    driver: "mysql",
    environment: "local",
    privacyTier: "local",
    readOnly: false,
    capabilities: ["TABLES", "SQL"],
    console: {
      session,
      capabilities: ["TABLES", "SQL"],
      readOnly: false,
      transactionState: "idle",
    },
  } as OpenConnection
  return render(
    <StrictMode>
      <QueryClientProvider client={client}>
        <CatalogSidebar
          open={open}
          sessionScoped
          selected={null}
          onSelect={() => undefined}
          onLeave={() => undefined}
        />
      </QueryClientProvider>
    </StrictMode>
  )
}

beforeEach(() => {
  ipc.reads.length = 0
  ipc.next = 0
  ipc.cancel.mockClear()
})
afterEach(async () => {
  cleanup()
  await act(async () => {
    await metadataReads.cancelCatalog("connection", "console-a")
    await metadataReads.cancelCatalog("connection", "console-b")
    for (const read of ipc.reads) read.resolve({ type: "done" })
    await Promise.all(ipc.reads.map((read) => read.outcome))
  })
})

it("keeps one cancellable source read across StrictMode and sidebar navigation", async () => {
  const first = show()
  await waitFor(() => expect(ipc.reads).toHaveLength(1))
  expect(ipc.cancel).not.toHaveBeenCalled()
  expect(ipc.reads[0]).toMatchObject({
    connection: "connection",
    session: "console-a",
  })
  first.unmount()
  expect(ipc.cancel).not.toHaveBeenCalled()
  show()
  await waitFor(() =>
    expect(
      screen
        .getByRole("button", { name: "Cancel catalog" })
        .hasAttribute("disabled")
    ).toBe(false)
  )
  expect(ipc.reads).toHaveLength(1)
  fireEvent.click(screen.getByRole("button", { name: "Cancel catalog" }))
  expect(ipc.cancel).toHaveBeenCalledExactlyOnceWith(ipc.reads[0]?.id)
  await act(async () => {
    ipc.reads[0]?.resolve({ type: "cancelled" })
  })
  expect(
    await screen.findByText("Refresh cancelled. The tree shows the last read.")
  ).toBeTruthy()
})

it("keeps distinct owners separate while an old sidebar read finishes", async () => {
  const first = show()
  first.unmount()
  show("console-b")
  await waitFor(() => expect(ipc.reads).toHaveLength(2))
  expect(ipc.reads.map((read) => read.session)).toEqual([
    "console-a",
    "console-b",
  ])
  expect(ipc.cancel).not.toHaveBeenCalled()
})

it("keeps Cancel available when an old root finishes before its queued replacement", async () => {
  show()
  await waitFor(() => expect(ipc.reads).toHaveLength(1))
  act(() => ipc.emit({ type: "catalogInvalidated", connection: "connection" }))
  await act(async () => {
    ipc.reads[0]?.resolve({ type: "done" })
  })
  await waitFor(() => expect(ipc.reads).toHaveLength(2))
  const stop = screen.getByRole("button", { name: "Cancel catalog" })
  expect(stop.hasAttribute("disabled")).toBe(false)
  fireEvent.click(stop)
  expect(ipc.cancel).toHaveBeenCalledExactlyOnceWith(ipc.reads[1]?.id)
  await act(async () => {
    ipc.reads[1]?.resolve({ type: "cancelled" })
  })
  await waitFor(() => expect(stop.hasAttribute("disabled")).toBe(true))
})

it("explicit cancellation prevents the owed refresh pass from starting", async () => {
  show()
  await waitFor(() => expect(ipc.reads).toHaveLength(1))
  act(() => {
    ipc.emit({ type: "catalogInvalidated", connection: "connection" })
    ipc.emit({ type: "catalogInvalidated", connection: "connection" })
  })
  fireEvent.click(screen.getByRole("button", { name: "Cancel catalog" }))
  await act(async () => {
    ipc.reads[0]?.resolve({ type: "cancelled" })
  })
  await waitFor(() =>
    expect(
      screen
        .getByRole("button", { name: "Cancel catalog" })
        .hasAttribute("disabled")
    ).toBe(true)
  )
  expect(ipc.reads).toHaveLength(1)
  expect(ipc.cancel).toHaveBeenCalledExactlyOnceWith(ipc.reads[0]?.id)
})
