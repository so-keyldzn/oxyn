import { act, renderHook, waitFor } from "@testing-library/react"
import { describe, expect, it, vi } from "vitest"

import { useConsoles } from "./use-consoles"
import type { ConsoleHandle } from "./console-panel"
import type { OpenConnection } from "@/lib/ipc/types"

vi.mock("@/lib/ipc/library", () => ({
  library: { newDocument: () => Promise.resolve("doc-1") },
}))

// Its schemas import the mocked `consoles` module; nothing here moves a tab.
vi.mock("@/lib/ipc/windows", () => ({
  windows: { openInNewWindow: vi.fn(), reportConsoles: vi.fn() },
}))
vi.mock("@/lib/ipc/consoles", () => ({
  consoles: { close: () => Promise.resolve() },
}))

vi.mock("@/lib/ipc/client", () => ({
  BackendError: class extends Error {},
  backend: { cancel: () => Promise.resolve() },
  newCommandId: () => "command",
}))

const open: OpenConnection = {
  connection: "conn-1",
  session: "catalog-1",
  name: "Orders",
  driver: "postgres",
  environment: "development",
  readOnly: false,
  privacyTier: "metadata",
  capabilities: [],
  console: {
    session: "console-1",
    capabilities: [],
    readOnly: false,
    transactionState: "idle",
  },
}

/** A modified console whose named save waits for the test to answer it. */
function modifiedConsole() {
  let text = "SELECT 1"
  const saves: Array<(saved: boolean) => void> = []
  const handle = {
    closeReasons: () => ({
      title: "Console 1",
      connection: "Orders",
      conflict: false,
      hasSavedCopy: true,
      unsaved: true,
      running: false,
      transaction: null,
    }),
    handoff: () => ({ result: null, parameters: [] }),
    text: () => text,
    flush: () => Promise.resolve(true),
    save: () =>
      new Promise<boolean>((resolve) => {
        saves.push(resolve)
      }),
    close: vi.fn(() => Promise.resolve(true)),
    cancelWrite: vi.fn(),
    insert: (sql: string) => {
      text = sql
    },
    focus: () => undefined,
    rename: () => undefined,
  } satisfies ConsoleHandle
  return { handle, saves }
}

async function openConsoles() {
  const hook = renderHook(() => useConsoles({ open, onActivate: () => {} }))
  await waitFor(() => expect(hook.result.current.entries).toHaveLength(1))
  const key = hook.result.current.entries[0]?.key ?? ""
  const console = modifiedConsole()
  hook.result.current.register(key, console.handle)
  return { hook, key, ...console }
}

/** Chooses Save and close, and returns once the save is sent, not answered. */
async function startSave(
  hook: Awaited<ReturnType<typeof openConsoles>>["hook"]
) {
  await act(async () => {
    void hook.result.current.decideClose("save")
  })
  expect(hook.result.current.closing?.busy).toBe(true)
}

describe("closing a modified console", () => {
  it("keeps the console open when a cancelled save answers late", async () => {
    const { hook, key, handle, saves } = await openConsoles()
    act(() => {
      // Settles when the close is decided; these tests decide it themselves.
      void hook.result.current.requestClose(key)
    })
    expect(hook.result.current.closing?.key).toBe(key)

    await startSave(hook)
    await act(() => hook.result.current.decideClose("cancel"))
    expect(hook.result.current.closing).toBeNull()
    expect(handle.cancelWrite).toHaveBeenCalledOnce()

    // Typed after Cancel, before the store answers the save.
    handle.insert("SELECT 2")
    await act(async () => saves[0]?.(true))

    expect(hook.result.current.entries.map((entry) => entry.key)).toEqual([key])
    expect(handle.close).not.toHaveBeenCalled()
    expect(hook.result.current.closing).toBeNull()
    expect(hook.result.current.handles.current.get(key)?.text()).toBe(
      "SELECT 2"
    )
  })

  it("does not let a late answer settle a newer close request", async () => {
    const { hook, key, handle, saves } = await openConsoles()
    act(() => {
      // Settles when the close is decided; these tests decide it themselves.
      void hook.result.current.requestClose(key)
    })
    await startSave(hook)
    await act(() => hook.result.current.decideClose("cancel"))

    act(() => {
      // Settles when the close is decided; these tests decide it themselves.
      void hook.result.current.requestClose(key)
    })
    const request = hook.result.current.closing
    expect(request).toEqual(expect.objectContaining({ key, busy: false }))

    await act(async () => saves[0]?.(true))
    expect(handle.close).not.toHaveBeenCalled()
    expect(hook.result.current.entries).toHaveLength(1)
    expect(hook.result.current.closing).toBe(request)
  })
})
