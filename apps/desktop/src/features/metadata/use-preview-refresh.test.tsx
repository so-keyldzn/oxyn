import { QueryClient, QueryClientProvider } from "@tanstack/react-query"
import { act, cleanup, renderHook, waitFor } from "@testing-library/react"
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest"

import type { PreviewShape, RefreshSignal } from "@/lib/ipc/metadata"
import type { CommandOutcome, OpenConnection } from "@/lib/ipc/types"

// The backend a preview reads through, answered by hand: the test counts the
// reads that reach it and releases each answer itself.
const ipc = vi.hoisted(() => {
  const pending = new Map<string, (outcome: CommandOutcome) => void>()
  const state = {
    pending,
    retained: false,
    next: 0,
    emit: (_signal: RefreshSignal): void => undefined,
    cancelled: [] as Array<string>,
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
      previewPagination: () =>
        ipc.retained
          ? Promise.resolve({
              type: "retained",
              previous: true,
              next: true,
              firstRow: 1,
            })
          : new Promise(() => undefined),
      subscribeRefreshSignals: (onSignal: (signal: RefreshSignal) => void) => {
        ipc.emit = onSignal
        return Promise.resolve()
      },
    },
  }
})
vi.mock("@/lib/ipc/client", () => ({
  backend: {
    cancel: (id: string) => {
      ipc.cancelled.push(id)
      return Promise.resolve()
    },
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

const executed = (
  result: string
): Extract<CommandOutcome, { type: "executed" }> => ({
  type: "executed",
  affectedRows: null,
  result,
  columns: [{ name: "id", dataType: "Int64", nullable: false }],
  rows: 3,
  elapsedMs: 4,
  latencyMs: 4,
  complete: true,
  cancelled: false,
  truncated: false,
  rowLimitUnverified: false,
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
          described: null,
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
  ipc.retained = false
  ipc.reads = []
  ipc.cancelled = []
  ipc.pending.clear()
})

afterEach(() => cleanup())

describe("the size of a preview page", () => {
  it.each(["sqlite", "postgres", "mysql", "duckdb", "mongodb", "turso"])(
    "uses the same page controls for %s",
    async (driver) => {
      const open = { ...opened(), driver }
      const view = renderPreviews(open, ["paged"])
      await answerAll(executed)
      act(() => view.result.current.at(0)?.resizePage(50))
      await answerAll((id) => ({ ...executed(id), rows: 50 }))
      act(() => view.result.current.at(0)?.page(true))
      expect(ipc.reads.at(-1)).toMatchObject({
        session: open.session,
        shape: { pageSize: 50, offset: 50 },
      })
      await answerAll(executed)
    }
  )

  it("pages by the selected size and keeps the applied filter and sort", async () => {
    const view = renderPreviews(opened(), ["sized"])
    await answerAll((id) => ({ ...executed(id), rows: 200 }))
    const preview = () => {
      const value = view.result.current.at(0)
      if (!value) throw new Error("one preview is mounted")
      return value
    }
    act(() => preview().resizePage(50))
    expect(ipc.reads.at(-1)?.shape).toMatchObject({ pageSize: 50, offset: 0 })
    await answerAll((id) => ({ ...executed(id), rows: 50 }))
    act(() => preview().page(true))
    expect(ipc.reads.at(-1)?.shape).toMatchObject({ pageSize: 50, offset: 50 })
    await answerAll((id) => ({ ...executed(id), rows: 1 }))
    act(() => preview().page(false))
    expect(ipc.reads.at(-1)?.shape).toMatchObject({ pageSize: 50, offset: 0 })
    await answerAll((id) => ({ ...executed(id), rows: 50 }))
    act(() => preview().applyPredicate(" id >= 2 "))
    await answerAll(executed)
    const sort = [{ column: "id", descending: true }]
    act(() => preview().applySort(sort))
    await answerAll(executed)
    act(() => preview().resizePage(100))
    expect(ipc.reads.at(-1)?.shape).toEqual({
      pageSize: 100,
      continuation: null,
      offset: 0,
      predicate: " id >= 2 ",
      sort,
    })
    await answerAll(executed)
    view.show([])
    const count = ipc.reads.length
    view.show(["sized"])
    expect(ipc.reads).toHaveLength(count)
    expect(preview().applied.pageSize).toBe(100)
  })

  it("ignores a late page of another size and keeps connections independent", async () => {
    const first = renderPreviews(opened(), ["sized"])
    await answerAll(executed)
    act(() => first.result.current.at(0)?.resizePage(50))
    const old = [...ipc.pending.values()]
    ipc.pending.clear()
    act(() => first.result.current.at(0)?.resizePage(100))
    await answerAll(executed)
    await act(async () => {
      for (const resolve of old) resolve({ ...executed("late"), rows: 50 })
    })
    expect(first.result.current.at(0)?.applied.pageSize).toBe(100)
    const second = renderPreviews(opened(), ["sized"])
    expect(ipc.reads.at(-1)?.shape.pageSize).toBe(200)
    await answerAll(executed)
    expect(second.result.current.at(0)?.applied.pageSize).toBe(200)
    expect(first.result.current.at(0)?.applied.pageSize).toBe(100)
  })
})

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

describe("an automatic read on a tab that is no longer shown", () => {
  it("is cancelled, so the next object's read is not queued behind it", async () => {
    const open = opened()
    const view = renderPreviews(open, ["slow", "small"])
    // Both tabs read on mount; only `slow` is meant to be on screen.
    view.show(["slow"])
    const slow = ipc.reads.find((read) => read.relation === "slow")
    expect(slow).toBeDefined()
    // `small` read on mount too, and was cancelled when `slow` took the screen.
    ipc.cancelled = []

    view.show(["small"])
    expect(ipc.cancelled).toEqual([slow?.id])
    expect(
      Object.values(previews.store.state).find(
        (entry) => entry.address.relation === "slow"
      )
    ).toMatchObject({ running: null, state: { status: "initial" } })
  })

  it("reads again on return, and never shows the answer that came late", async () => {
    const open = opened()
    const view = renderPreviews(open, ["slow", "small"])
    view.show(["slow"])
    const first = ipc.reads.find((read) => read.relation === "slow")?.id ?? ""

    view.show(["small"])
    const mark = ipc.reads.length
    view.show(["slow"])
    const second = readsSince(mark).find((read) => read.relation === "slow")
    expect(second).toBeDefined()
    expect(second?.id).not.toBe(first)

    // The abandoned read answers last: its rows are released, not shown.
    ipc.pending.get(first)?.(executed("late"))
    await act(async () => {
      await Promise.resolve()
    })
    ipc.pending.get(second?.id ?? "")?.(executed("fresh"))
    await act(async () => {
      await Promise.resolve()
    })
    // `slow` is the first relation rendered.
    expect(view.result.current[0]?.state).toMatchObject({
      status: "populated",
      result: "fresh",
    })
  })

  it("keeps a loaded preview cached across the tab change", async () => {
    const open = opened()
    const view = renderPreviews(open, ["a", "b"])
    view.show(["a", "b"])
    await answerAll((id) => executed(`r-${id}`))

    const mark = ipc.reads.length
    view.show(["b"])
    view.show(["a"])
    expect(ipc.cancelled).toEqual([])
    expect(readsSince(mark)).toEqual([])
    expect(view.result.current[0]?.state).toMatchObject({
      status: "populated",
    })
  })

  it("leaves a read the user asked for running", async () => {
    const open = opened()
    const view = renderPreviews(open, ["a", "b"])
    view.show(["a"])
    await answerAll((id) => executed(`r-${id}`))
    ipc.cancelled = []
    act(() => view.result.current[0]?.applyPredicate("id > 3"))
    expect(view.result.current[0]?.running).toBe(true)

    view.show(["b"])
    expect(ipc.cancelled).toEqual([])
    expect(view.result.current[0]?.running).toBe(true)
  })
})

it.each(["clickhouse", "elasticsearch", "redis"])(
  "keeps only %s navigation on its retained read",
  async (driver) => {
    ipc.retained = true
    const open = { ...opened(), driver }
    const view = renderPreviews(open, ["native"])
    const preview = () => {
      const value = view.result.current.at(0)
      if (!value) throw new Error("one preview is mounted")
      return value
    }
    await answerAll(() => ({ ...executed("first"), rows: 200 }))
    await waitFor(() => expect(preview().pagination?.type).toBe("retained"))
    act(() => preview().page(true))
    expect(ipc.reads.at(-1)?.shape).toMatchObject({
      offset: 200,
      continuation: "first",
    })
    await answerAll(() => ({ ...executed("second"), rows: 200 }))
    await waitFor(() => expect(preview().pagination?.type).toBe("retained"))
    act(() => preview().page(false))
    expect(ipc.reads.at(-1)?.shape).toMatchObject({
      offset: 0,
      continuation: "second",
    })
    await answerAll(() => ({ ...executed("back"), rows: 200 }))
    await waitFor(() => expect(preview().pagination?.type).toBe("retained"))
    act(() => preview().refresh())
    expect(ipc.reads.at(-1)?.shape).toMatchObject({
      offset: 0,
      continuation: null,
    })
    await answerAll(executed)
    act(() => preview().resizePage(50))
    expect(ipc.reads.at(-1)?.shape).toMatchObject({
      offset: 0,
      pageSize: 50,
      continuation: null,
    })
    await answerAll(executed)
  }
)
