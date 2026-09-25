import * as React from "react"
import { QueryClient, QueryClientProvider } from "@tanstack/react-query"
import { act, cleanup, renderHook } from "@testing-library/react"
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest"

import { DRAFT_IDLE_MS, useConsoleDocument } from "./use-console-document"
import type { ConsoleSeed } from "./use-console-document"
import type { DocumentWrite } from "@/lib/ipc/library"

interface PendingWrite {
  named: boolean
  text: string
  answer: (write: DocumentWrite) => void
}

const store = vi.hoisted(() => ({ writes: [] as Array<PendingWrite> }))

vi.mock("@/lib/ipc/library", () => ({
  library: {
    saveDocument: (_id: string, request: { named: boolean; text: string }) =>
      new Promise<DocumentWrite>((resolve) => {
        store.writes.push({
          named: request.named,
          text: request.text,
          answer: resolve,
        })
      }),
  },
}))

vi.mock("@/lib/ipc/client", () => ({
  backend: { cancel: () => Promise.resolve() },
  newCommandId: () => "command",
}))

const seed: ConsoleSeed = {
  document: "doc-1",
  revision: 0,
  title: "Draft",
  text: "",
  savedTitle: null,
  savedText: null,
  hasSavedCopy: false,
  fromAgent: false,
  parameters: [],
  needsValues: false,
}

const saved: DocumentWrite = {
  type: "saved",
  revision: 1,
  savedRevision: 0,
  isSaved: false,
}

function open() {
  const client = new QueryClient({
    defaultOptions: { queries: { retry: false } },
  })
  return renderHook(() => useConsoleDocument({ seed, connection: "c" }), {
    wrapper: ({ children }: { children: React.ReactNode }) => (
      <QueryClientProvider client={client}>{children}</QueryClientProvider>
    ),
  })
}

/** Starts a draft write and returns once it is sent, not answered. */
async function start(hook: ReturnType<typeof open>) {
  await act(async () => {
    void hook.result.current.flush()
  })
}

async function answer(index: number, write: DocumentWrite) {
  await act(async () => {
    store.writes[index]?.answer(write)
  })
}

// Without globals, Testing Library does not unmount on its own: a hook left
// mounted keeps its draft timer, which then fires once the file's environment
// is gone and fails the whole run with `window is not defined`.
afterEach(() => {
  cleanup()
  vi.useRealTimers()
})

beforeEach(() => {
  store.writes = []
})

describe("the recovery draft timer", () => {
  it("writes a pending draft once typing pauses", () => {
    vi.useFakeTimers()
    const hook = open()
    act(() => hook.result.current.change({ text: "SELECT 1" }))
    act(() => vi.advanceTimersByTime(DRAFT_IDLE_MS))
    expect(store.writes.map((write) => write.text)).toEqual(["SELECT 1"])
  })

  it("does not outlive the console", () => {
    vi.useFakeTimers()
    const hook = open()
    act(() => hook.result.current.change({ text: "SELECT 1" }))
    hook.unmount()
    vi.advanceTimersByTime(DRAFT_IDLE_MS)
    expect(store.writes).toHaveLength(0)
  })
})

describe("the recovery draft notice", () => {
  it("says the draft is not saved when the name is out of bounds", async () => {
    const hook = open()
    act(() => hook.result.current.change({ title: "x".repeat(257) }))
    await act(() => hook.result.current.flush())
    expect(store.writes).toHaveLength(0)
    expect(hook.result.current.draftNotice).toMatch(/^Draft not saved: /)
  })

  it("announces recovery only for the text the editor still shows", async () => {
    const stale = open()
    act(() => stale.result.current.change({ text: "SELECT 1" }))
    await start(stale)
    act(() => stale.result.current.change({ text: "SELECT 12" }))
    await answer(0, saved)
    expect(stale.result.current.draftNotice).not.toMatch(/saved locally/)

    const fresh = open()
    act(() => fresh.result.current.change({ text: "SELECT 2" }))
    await start(fresh)
    await answer(1, saved)
    expect(fresh.result.current.draftNotice).toBe(
      "Recovery draft saved locally."
    )
  })

  it("does not keep « saving » once the draft was superseded", async () => {
    const hook = open()
    act(() => hook.result.current.change({ text: "SELECT 1" }))
    await start(hook)
    expect(hook.result.current.draftNotice).toBe("Saving recovery draft…")
    await answer(0, { type: "superseded" })
    expect(hook.result.current.draftNotice).not.toMatch(/Saving/)
  })

  it("lets only the last attempt speak", async () => {
    const hook = open()
    act(() => hook.result.current.change({ text: "SELECT 1" }))
    await start(hook)
    act(() => hook.result.current.change({ text: "SELECT 12" }))
    await start(hook)
    await answer(1, saved)
    await answer(0, { type: "superseded" })
    expect(hook.result.current.draftNotice).toBe(
      "Recovery draft saved locally."
    )
  })
})
