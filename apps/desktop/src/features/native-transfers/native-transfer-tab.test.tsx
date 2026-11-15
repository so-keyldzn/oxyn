import * as React from "react"
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
import { NativeTransferTab } from "./native-transfer-tab"
import type { NativeTransferHandle } from "./native-transfer-tab"
import type { NativeTransferView } from "@/components/oxyn/native-transfer-view"
import {
  nativeJobFixture,
  nativeReviewFixture,
  nativeToolFixture,
} from "@/components/oxyn/native-transfer-fixtures"
import type { OpenConnection } from "@/lib/ipc/types"

const ipc = vi.hoisted(() => ({
  next: 0,
  cancel: vi.fn(async (_id: string) => undefined),
  revoke: vi.fn(async () => undefined),
  support: vi.fn(),
  jobs: vi.fn(),
  tool: vi.fn(),
  file: vi.fn(),
  prepare: vi.fn(),
  start: vi.fn(),
  discard: vi.fn(),
}))
vi.mock("@/lib/ipc/client", () => ({
  BackendError: class extends Error {
    retryable = false
  },
  backend: { cancel: ipc.cancel },
  newCommandId: () => `native-${++ipc.next}`,
}))
vi.mock("@/lib/ipc/native-transfers", () => ({
  nativeTransfers: {
    support: ipc.support,
    jobs: ipc.jobs,
    selectTool: ipc.tool,
    selectFile: ipc.file,
    prepare: ipc.prepare,
    start: ipc.start,
    discard: ipc.discard,
  },
  nativeSelections: { revoke: ipc.revoke, progress: async () => null },
}))
vi.mock("@/components/oxyn/native-transfer-view", () => ({
  NativeTransferView: (
    props: React.ComponentProps<typeof NativeTransferView>
  ) => (
    <>
      <button onClick={() => props.onOperation("restore")}>Restore mode</button>
      <button onClick={props.onTool}>Tool</button>
      <button onClick={props.onFile}>File</button>
      <button onClick={() => props.onTrust(true)}>Trust</button>
      <p>{props.state.trustedArchive ? "Trusted" : "Untrusted"}</p>
      <button onClick={props.onPrepare}>Prepare</button>
      <button onClick={props.onStart}>Start</button>
      <p>{props.state.busy ?? "Idle"}</p>
      <p>{props.state.file?.name}</p>
      <p>{props.state.review ? "Prepared" : "Unprepared"}</p>
    </>
  ),
}))
vi.mock("@/components/oxyn/native-transfer-stop-review", () => ({
  NativeTransferStopReview: ({
    onKeep,
    onStop,
  }: {
    onKeep: () => void
    onStop: () => void
  }) => (
    <>
      <button onClick={onKeep}>Keep</button>
      <button onClick={onStop}>Stop and close</button>
    </>
  ),
}))
const owner = {
  connection: "fixture",
  session: "owner",
  name: "Fixture",
} as OpenConnection
function show() {
  const ref = React.createRef<NativeTransferHandle>()
  const client = new QueryClient({
    defaultOptions: { queries: { retry: false } },
  })
  return {
    ...render(
      <QueryClientProvider client={client}>
        <NativeTransferTab open={owner} handleRef={ref} />
      </QueryClientProvider>
    ),
    ref,
  }
}
async function select() {
  fireEvent.click(screen.getByText("Tool"))
  await waitFor(() => expect(screen.getByText("Idle")).toBeTruthy())
  fireEvent.click(screen.getByText("File"))
  await waitFor(() => expect(screen.getByText("fixture.dump")).toBeTruthy())
}
beforeEach(() => {
  vi.clearAllMocks()
  ipc.support.mockResolvedValue({
    engine: "postgres",
    export: true,
    restore: true,
    reason: null,
    exportFormats: ["customArchive", "sql"],
    restoreFormats: ["customArchive"],
  })
  ipc.jobs.mockResolvedValue([])
  ipc.tool.mockResolvedValue(nativeToolFixture)
  ipc.file.mockResolvedValue({ grant: "file-grant", name: "fixture.dump" })
  ipc.prepare.mockResolvedValue(nativeReviewFixture)
  ipc.discard.mockResolvedValue(nativeJobFixture({ outcome: "refused" }))
  ipc.start.mockResolvedValue(nativeJobFixture())
})
afterEach(cleanup)
it("never prepares or runs on mount and resets archive trust for a new file", async () => {
  show()
  await waitFor(() => expect(ipc.support).toHaveBeenCalled())
  expect(ipc.prepare).not.toHaveBeenCalled()
  expect(ipc.start).not.toHaveBeenCalled()
  fireEvent.click(screen.getByText("Restore mode"))
  fireEvent.click(screen.getByText("Trust"))
  expect(screen.getByText("Trusted")).toBeTruthy()
  fireEvent.click(screen.getByText("File"))
  await waitFor(() => expect(screen.getByText("Untrusted")).toBeTruthy())
  expect(ipc.file).toHaveBeenCalledWith(
    "fixture",
    "owner",
    "restore",
    "customArchive"
  )
})
it("releases a tool returned after its original retained controller unmounts", async () => {
  let resolve!: (tool: typeof nativeToolFixture) => void
  ipc.tool.mockReturnValue(
    new Promise((done) => {
      resolve = done
    })
  )
  const view = show()
  fireEvent.click(screen.getByText("Tool"))
  const command = ipc.tool.mock.calls[0]?.[0]
  view.unmount()
  expect(ipc.cancel).toHaveBeenCalledWith(command)
  await act(async () => resolve(nativeToolFixture))
  expect(ipc.revoke).toHaveBeenCalledWith(
    expect.any(String),
    nativeToolFixture.grant
  )
})
it("closing an active job waits for cancellation and settled cleanup", async () => {
  let finish!: (job: ReturnType<typeof nativeJobFixture>) => void
  ipc.start.mockReturnValue(
    new Promise((done) => {
      finish = done
    })
  )
  const view = show()
  await select()
  fireEvent.click(screen.getByText("Prepare"))
  await waitFor(() => expect(screen.getByText("Prepared")).toBeTruthy())
  fireEvent.click(screen.getByText("Start"))
  await waitFor(() => expect(screen.getByText("starting")).toBeTruthy())
  let closed = false
  let closing!: Promise<boolean>
  act(() => {
    closing = view.ref.current!.requestClose()
    void closing.then((result) => {
      closed = result
    })
  })
  fireEvent.click(screen.getByText("Stop and close"))
  await waitFor(() => expect(ipc.cancel).toHaveBeenCalled())
  expect(closed).toBe(false)
  expect(ipc.discard).not.toHaveBeenCalled()
  await act(async () => {
    finish(nativeJobFixture({ outcome: "cancelled" }))
    await closing
  })
  expect(closed).toBe(true)
  expect(ipc.start).toHaveBeenCalledTimes(1)
})
it("discards a late preparation without starting it after unmount", async () => {
  let finish!: (review: typeof nativeReviewFixture) => void
  ipc.prepare.mockReturnValue(
    new Promise((done) => {
      finish = done
    })
  )
  const view = show()
  await select()
  fireEvent.click(screen.getByText("Prepare"))
  view.unmount()
  await act(async () => finish(nativeReviewFixture))
  expect(ipc.discard).toHaveBeenCalledWith(
    expect.any(String),
    "fixture",
    "owner",
    nativeReviewFixture.job.id
  )
  expect(ipc.start).not.toHaveBeenCalled()
})

it("releases a selection completed during closing before its batched view update commits", async () => {
  let finish!: (tool: typeof nativeToolFixture) => void
  ipc.tool.mockReturnValue(
    new Promise((resolve) => {
      finish = resolve
    })
  )
  const view = show()
  fireEvent.click(screen.getByText("Tool"))
  let closing!: Promise<boolean>
  act(() => {
    closing = view.ref.current!.requestClose()
  })
  fireEvent.click(screen.getByText("Stop and close"))
  await waitFor(() => expect(ipc.cancel).toHaveBeenCalled())
  await act(async () => {
    finish(nativeToolFixture)
    expect(await closing).toBe(true)
    view.unmount()
  })
  expect(ipc.revoke).toHaveBeenCalledWith(
    expect.any(String),
    nativeToolFixture.grant
  )
  expect(ipc.start).not.toHaveBeenCalled()
})

it("cancels its pending availability read on unmount without preparing a native job", async () => {
  let finish!: (value: unknown) => void
  ipc.support.mockReturnValue(
    new Promise((resolve) => {
      finish = resolve
    })
  )
  const view = show()
  await waitFor(() => expect(ipc.support).toHaveBeenCalled())
  const command = ipc.support.mock.calls[0]?.[0]
  view.unmount()
  await waitFor(() => expect(ipc.cancel).toHaveBeenCalledWith(command))
  await act(async () => {
    finish(null)
  })
  expect(ipc.prepare).not.toHaveBeenCalled()
  expect(ipc.start).not.toHaveBeenCalled()
})
