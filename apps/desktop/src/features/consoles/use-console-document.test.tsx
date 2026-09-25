import * as React from "react"
import { QueryClient, QueryClientProvider } from "@tanstack/react-query"
import { act, cleanup, renderHook } from "@testing-library/react"
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest"

import { DRAFT_IDLE_MS, useConsoleDocument } from "./use-console-document"
import type { ConsoleSeed } from "./use-console-document"
import type { DocumentWrite } from "@/lib/ipc/library"

interface PendingWrite {
  document: string
  named: boolean
  text: string
  answer: (write: DocumentWrite) => void
  fail: (error: Error) => void
}

const store = vi.hoisted(() => ({ writes: [] as Array<PendingWrite> }))

vi.mock("@/lib/ipc/library", () => ({
  library: {
    saveDocument: (
      _id: string,
      request: { document: string; named: boolean; text: string }
    ) =>
      new Promise<DocumentWrite>((resolve, reject) => {
        store.writes.push({
          document: request.document,
          named: request.named,
          text: request.text,
          answer: resolve,
          fail: reject,
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

function open(from: ConsoleSeed = seed) {
  const client = new QueryClient({
    defaultOptions: { queries: { retry: false } },
  })
  return renderHook(() => useConsoleDocument({ seed: from, connection: "c" }), {
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

/**
 * The writes of one document. Hooks of earlier tests stay mounted, and a
 * draft they still hold may be written during a later test.
 */
function writesOf(document: string) {
  return store.writes.filter((write) => write.document === document)
}

describe("the recovery draft", () => {
  it("writes a new copy's text before any edit", async () => {
    const copy = open({ ...seed, document: "copy", text: "SELECT 1" })
    let flushed: Promise<boolean> | undefined
    await act(async () => {
      flushed = copy.result.current.flush()
    })
    expect(writesOf("copy").map((write) => write.text)).toEqual(["SELECT 1"])
    await act(async () => writesOf("copy")[0]?.answer(saved))
    await expect(flushed).resolves.toBe(true)
  })

  it("writes a new copy's text after the typing pause, unflushed", async () => {
    vi.useFakeTimers()
    try {
      open({ ...seed, document: "idle-copy", text: "SELECT 1" })
      await act(async () => {
        vi.advanceTimersByTime(DRAFT_IDLE_MS)
      })
      expect(writesOf("idle-copy").map((write) => write.text)).toEqual([
        "SELECT 1",
      ])
    } finally {
      vi.useRealTimers()
    }
  })

  it("does not rewrite an empty console or a resumed document", async () => {
    const empty = open({ ...seed, document: "empty" })
    const resumed = open({
      ...seed,
      document: "resumed",
      revision: 4,
      text: "SELECT 1",
    })
    await act(async () => {
      await expect(empty.result.current.flush()).resolves.toBe(true)
      await expect(resumed.result.current.flush()).resolves.toBe(true)
    })
    expect(writesOf("empty")).toHaveLength(0)
    expect(writesOf("resumed")).toHaveLength(0)
  })

  it("sends a failed draft again once the store is back", async () => {
    const hook = open({ ...seed, document: "retried" })
    act(() => hook.result.current.change({ text: "SELECT 1" }))
    let first: Promise<boolean> | undefined
    await act(async () => {
      first = hook.result.current.flush()
    })
    await act(async () => writesOf("retried")[0]?.fail(new Error("disk full")))
    await expect(first).resolves.toBe(false)
    expect(hook.result.current.draftNotice).toBe("Draft not saved: disk full")

    let second: Promise<boolean> | undefined
    await act(async () => {
      second = hook.result.current.flush()
    })
    expect(writesOf("retried").map((write) => write.text)).toEqual([
      "SELECT 1",
      "SELECT 1",
    ])
    await act(async () => writesOf("retried")[1]?.answer(saved))
    await expect(second).resolves.toBe(true)
    expect(hook.result.current.draftNotice).toBe(
      "Recovery draft saved locally."
    )
  })

  it("lets a flush wait for the write already carrying its text", async () => {
    const hook = open({ ...seed, document: "awaited" })
    act(() => hook.result.current.change({ text: "SELECT 1" }))
    await start(hook)
    let flushed: Promise<boolean> | undefined
    await act(async () => {
      flushed = hook.result.current.flush()
    })
    expect(writesOf("awaited")).toHaveLength(1)
    await act(async () => writesOf("awaited")[0]?.fail(new Error("disk full")))
    await expect(flushed).resolves.toBe(false)
  })

  it("never confirms a draft frozen by a conflict", async () => {
    const hook = open({ ...seed, document: "conflicted" })
    act(() => hook.result.current.change({ text: "SELECT 1" }))
    let first: Promise<boolean> | undefined
    await act(async () => {
      first = hook.result.current.flush()
    })
    await act(async () =>
      writesOf("conflicted")[0]?.answer({ type: "conflict", message: "moved" })
    )
    await expect(first).resolves.toBe(false)
    await act(async () => {
      await expect(hook.result.current.flush()).resolves.toBe(false)
    })
    expect(writesOf("conflicted")).toHaveLength(1)
  })
})
