import { QueryClient, QueryClientProvider } from "@tanstack/react-query"
import { act, cleanup, renderHook } from "@testing-library/react"
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest"

import type { PreviewShape, RefreshSignal } from "@/lib/ipc/metadata"
import type { CommandOutcome, OpenConnection } from "@/lib/ipc/types"

// The backend a preview reads through, answered by hand: the test counts the
// reads that reach it and releases each answer itself.
const ipc = vi.hoisted(() => {
  const pending = new Map<string, (outcome: CommandOutcome) => void>()
  const state = {
    pending,
    next: 0,
    emit: (_signal: RefreshSignal): void => undefined,
    reads: [] as Array<{
      id: string
      relation: string
      session: string
      shape: PreviewShape
    }>,
  }
  return state
})

vi.mock("@/lib/ipc/metadata", async (importOriginal) => {
  const actual = await importOriginal<Record<string, unknown>>()
  return {
    ...actual,
    metadata: {
      previewRelation: (
        id: string,
        _connection: string,
        session: string,
        address: { relation: string },
        shape: PreviewShape
      ) => {
        ipc.reads.push({ id, relation: address.relation, session, shape })
        return new Promise((resolve) => ipc.pending.set(id, resolve))
      },
      previewPagination: () => new Promise(() => undefined),
      subscribeRefreshSignals: (onSignal: (signal: RefreshSignal) => void) => {
        ipc.emit = onSignal
        return Promise.resolve()
      },
    },
  }
})
vi.mock("@/lib/ipc/client", () => ({
  backend: {
    cancel: () => Promise.resolve(),
    decide: () => Promise.resolve(),
  },
  newCommandId: () => `cmd${++ipc.next}`,
}))
vi.mock("@/lib/ipc/results", () => ({
  results: {
    forgetResult: () => Promise.resolve(),
    resultColumns: () => Promise.resolve([]),
  },
}))

const { usePreview, previews } = await import("./use-preview")

const executed = (result: string): CommandOutcome => ({
  type: "executed",
  result,
  columns: [{ name: "id", dataType: "Int64", nullable: false }],
  rows: 3,
  elapsedMs: 4,
  complete: true,
  cancelled: false,
  truncated: false,
})

let connections = 0
function opened(): OpenConnection {
  connections++
  return {
    connection: `c${connections}`,
    session: `s${connections}`,
    capabilities: ["SQL", "SERVER_SIDE_CANCEL"],
  } as unknown as OpenConnection
}

const address = (relation: string) => ({
  catalog: null,
  namespace: "public",
  relation,
})

/** One preview per relation, each with its own visibility. */
function renderPreviews(open: OpenConnection, relations: Array<string>) {
  const client = new QueryClient({
    defaultOptions: { queries: { retry: false } },
  })
  const visible = new Map(relations.map((relation) => [relation, true]))
  const hook = renderHook(
    () =>
      relations.map((relation) =>
        usePreview({
          open,
          address: address(relation),
          enabled: true,
          visible: visible.get(relation) ?? false,
        })
      ),
    {
      wrapper: ({ children }) => (
        <QueryClientProvider client={client}>{children}</QueryClientProvider>
      ),
    }
  )
  return {
    ...hook,
    show: (shown: Array<string>) => {
      for (const relation of relations)
        visible.set(relation, shown.includes(relation))
      hook.rerender()
    },
  }
}

async function answerAll(outcome: (id: string) => CommandOutcome) {
  const waiting = [...ipc.pending]
  ipc.pending.clear()
  await act(async () => {
    for (const [id, resolve] of waiting) resolve(outcome(id))
    await Promise.resolve()
  })
}

function signal(value: RefreshSignal) {
  act(() => ipc.emit(value))
}

const readsSince = (mark: number) => ipc.reads.slice(mark)

beforeEach(() => {
  ipc.reads = []
})

afterEach(() => cleanup())

describe("after a write, only the preview on screen reads again", () => {
  it("counts one read for the visible tab, none for the hidden ones", async () => {
    const open = opened()
    const view = renderPreviews(open, ["a", "b", "c"])
    await answerAll((id) => executed(`r-${id}`))
    const sort = [{ column: "id", descending: true }]
    act(() => view.result.current[1]?.applySort(sort))
    await answerAll((id) => executed(`r-${id}`))

    // Several object tabs stay mounted; only `a` is the active one.
    view.show(["a"])
    const mark = ipc.reads.length
    signal({ type: "rowsChanged", connection: open.connection })

    expect(readsSince(mark).map((read) => read.relation)).toEqual(["a"])
    const hidden = Object.values(previews.store.state).filter(
      (entry) =>
        entry.connection === open.connection && entry.address.relation !== "a"
    )
    expect(hidden.map((entry) => entry.stale)).toEqual([true, true])
    await answerAll((id) => executed(`r-${id}`))

    // Back on `b`: one read, with the order it had.
    const back = ipc.reads.length
    view.show(["b"])
    expect(readsSince(back)).toMatchObject([
      { relation: "b", shape: { sort, offset: 0 } },
    ])
    await answerAll((id) => executed(`r-${id}`))
    view.show(["b"])
    expect(ipc.reads).toHaveLength(back + 1)
  })

  it("reads nothing while the whole workspace is hidden", async () => {
    const open = opened()
    const view = renderPreviews(open, ["a", "b"])
    await answerAll((id) => executed(`r-${id}`))

    view.show([])
    const mark = ipc.reads.length
    signal({ type: "rowsChanged", connection: open.connection })
    signal({ type: "lagged" })
    expect(readsSince(mark)).toEqual([])

    view.show(["a"])
    expect(readsSince(mark).map((read) => read.relation)).toEqual(["a"])
  })
})

describe("after a DDL", () => {
  it("reads the visible preview again, with the shape in force", async () => {
    const open = opened()
    const view = renderPreviews(open, ["invoices"])
    await answerAll((id) => executed(`r-${id}`))
    act(() => view.result.current[0]?.applyPredicate("total > 10"))
    await answerAll((id) => executed(`r-${id}`))

    // What the backend sends once `ALTER TABLE … ADD COLUMN` succeeded.
    const mark = ipc.reads.length
    signal({ type: "catalogInvalidated", connection: open.connection })
    expect(readsSince(mark)).toEqual([])
    signal({ type: "rowsChanged", connection: open.connection })

    expect(readsSince(mark)).toMatchObject([
      {
        relation: "invoices",
        session: open.session,
        shape: { predicate: "total > 10" },
      },
    ])
    await answerAll((id) => executed(`after-${id}`))
    expect(view.result.current[0]?.state).toMatchObject({
      status: "populated",
      result: `after-${readsSince(mark)[0]?.id}`,
    })
  })

  it("leaves a hidden preview stale until it is shown", async () => {
    const open = opened()
    const view = renderPreviews(open, ["invoices"])
    await answerAll((id) => executed(`r-${id}`))
    view.show([])

    const mark = ipc.reads.length
    signal({ type: "catalogInvalidated", connection: open.connection })
    signal({ type: "rowsChanged", connection: open.connection })
    expect(readsSince(mark)).toEqual([])

    view.show(["invoices"])
    expect(readsSince(mark)).toHaveLength(1)
  })

  it("never hides an error behind a new read", async () => {
    const open = opened()
    const view = renderPreviews(open, ["invoices"])
    await answerAll(() => ({ type: "denied", reason: "permission denied" }))
    const failed = view.result.current[0]?.state

    const mark = ipc.reads.length
    signal({ type: "rowsChanged", connection: open.connection })
    view.show([])
    view.show(["invoices"])

    expect(readsSince(mark)).toEqual([])
    expect(failed).toMatchObject({ status: "error" })
    expect(view.result.current[0]?.state).toEqual(failed)
  })

  it("reads nothing for a DDL on another connection", async () => {
    const open = opened()
    renderPreviews(open, ["invoices"])
    await answerAll((id) => executed(`r-${id}`))

    const mark = ipc.reads.length
    signal({ type: "rowsChanged", connection: "elsewhere" })
    expect(readsSince(mark)).toEqual([])
  })
})
