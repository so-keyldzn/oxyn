import { QueryClient, QueryClientProvider } from "@tanstack/react-query"
import {
  cleanup,
  fireEvent,
  render,
  screen,
  waitFor,
} from "@testing-library/react"
import { afterEach, expect, it, vi } from "vitest"
import { MetadataLoadingSettings } from "./loading-settings"
import type { CatalogNode, OpenConnection } from "@/lib/ipc/types"
import type { MetadataPreloadProgress } from "@/lib/ipc/metadata-loading"

const ipc = vi.hoisted(() => ({
  read: vi.fn(),
  save: vi.fn(),
  preload: vi.fn(),
  progress: vi.fn(),
  cancel: vi.fn(),
  sequence: 0,
}))
vi.mock("@/lib/ipc/metadata-loading", () => ({ metadataLoading: ipc }))
vi.mock("@/lib/ipc/client", () => ({
  newCommandId: () => `command-${++ipc.sequence}`,
  backend: { cancel: ipc.cancel },
  BackendError: class extends Error {
    retryable = false
  },
}))
vi.stubGlobal(
  "ResizeObserver",
  class {
    observe() {}
    unobserve() {}
    disconnect() {}
  }
)
// jsdom has no Web Animations API; real overlay behavior is covered in Chromium stories.
Object.defineProperty(Element.prototype, "getAnimations", {
  configurable: true,
  value: () => [],
})

const policy = { version: 1, default: "minimal", overrides: [] }
const open: OpenConnection = {
  connection: "one",
  session: "one-session",
  name: "First source",
  driver: "sqlite",
  environment: "local",
  readOnly: true,
  privacyTier: "metadata",
  capabilities: ["SCHEMAS"],
  console: {
    session: "console",
    capabilities: [],
    readOnly: true,
    transactionState: "idle",
  },
}
const nodes: Array<CatalogNode> = [
  {
    address: { catalog: null, namespace: "main", relation: null },
    name: "main",
    kind: "namespace",
    holdsRecords: false,
    system: false,
    comment: null,
    loaded: true,
    stale: false,
    children: [],
    virtualTable: null,
    shadowOf: null,
  },
]
afterEach(() => {
  cleanup()
  vi.resetAllMocks()
})
function setup() {
  ipc.read.mockResolvedValue({ revision: 0, policy })
  ipc.save.mockResolvedValue({ revision: 1, policy })
  ipc.progress.mockResolvedValue(null)
  ipc.cancel.mockResolvedValue(undefined)
  const client = new QueryClient({
    defaultOptions: { queries: { retry: false, networkMode: "always" } },
  })
  const show = (source: OpenConnection) => (
    <QueryClientProvider client={client}>
      <MetadataLoadingSettings open={source} nodes={nodes} />
    </QueryClientProvider>
  )
  const rendered = render(show(open))
  fireEvent.click(screen.getByRole("button", { name: "Metadata preferences…" }))
  return { ...rendered, show }
}
it("saves a loading preference without starting any metadata preload", async () => {
  setup()
  await waitFor(() =>
    expect(
      screen.getByRole("button", { name: "Save preferences" })
    ).toHaveProperty("disabled", false)
  )
  fireEvent.click(screen.getByRole("button", { name: "Extended" }))
  fireEvent.click(screen.getByRole("button", { name: "Save preferences" }))
  await waitFor(() =>
    expect(ipc.save).toHaveBeenCalledWith(
      expect.any(String),
      "one",
      { ...policy, default: "extended" },
      0
    )
  )
  expect(ipc.preload).not.toHaveBeenCalled()
})
it("cancels the old source command and ignores its late completion after source replacement", async () => {
  let finish!: (report: MetadataPreloadProgress) => void
  ipc.preload.mockImplementation(
    () =>
      new Promise<MetadataPreloadProgress>((resolve) => {
        finish = resolve
      })
  )
  const { rerender, show } = setup()
  fireEvent.click(
    await screen.findByRole("button", { name: "Advanced: optional preload" })
  )
  await waitFor(() =>
    expect(
      screen.getByRole("button", { name: "Preload scope" })
    ).toHaveProperty("disabled", false)
  )
  fireEvent.click(screen.getByRole("button", { name: "Preload scope" }))
  await waitFor(() => expect(ipc.preload).toHaveBeenCalledOnce())
  const id = ipc.preload.mock.calls[0]?.[0] as string
  rerender(
    show({
      ...open,
      connection: "two",
      session: "two-session",
      name: "Second source",
    })
  )
  await waitFor(() => expect(ipc.cancel).toHaveBeenCalledWith(id))
  finish({
    command: id,
    connection: "one",
    session: "one-session",
    state: "complete",
  } as MetadataPreloadProgress)
  await waitFor(() => expect(screen.getByText("Second source")).not.toBeNull())
  expect(screen.queryByText(/encoded metadata bytes/)).toBeNull()
  expect(ipc.preload).toHaveBeenCalledOnce()
})
it("requires re-reading after a stale save while keeping the dialog dismissible", async () => {
  setup()
  ipc.save.mockRejectedValue(new Error("Preferences changed in another window"))
  await waitFor(() =>
    expect(
      screen.getByRole("button", { name: "Save preferences" })
    ).toHaveProperty("disabled", false)
  )
  fireEvent.click(screen.getByRole("button", { name: "Save preferences" }))
  await screen.findByText("Preferences changed in another window")
  expect(
    screen.getByRole("button", { name: "Save preferences" })
  ).toHaveProperty("disabled", true)
  expect(screen.getByRole("button", { name: "Close" })).toHaveProperty(
    "disabled",
    false
  )
  fireEvent.click(
    screen.getByRole("button", { name: "Read saved preferences" })
  )
  await waitFor(() =>
    expect(
      screen.getByRole("button", { name: "Save preferences" })
    ).toHaveProperty("disabled", false)
  )
  expect(ipc.preload).not.toHaveBeenCalled()
})
