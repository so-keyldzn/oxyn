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
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest"

import { ToolRows } from "./tool-rows"
import type { ToolCallEntry } from "./transcript"
import type * as ResultGridModule from "@/components/oxyn/result-grid"
import type { FetchPage } from "@/components/oxyn/result-grid"
import type { RetainedResult } from "@/lib/ipc/library"
import type { ResultWindow } from "@/lib/ipc/types"

const backend = vi.hoisted(() => ({
  openRetainedResult: vi.fn<(connection: string, result: string) => unknown>(),
  readResultPage:
    vi.fn<
      (
        connection: string,
        result: string,
        offset: number,
        limit: number
      ) => Promise<unknown>
    >(),
  forgetResult: vi.fn(() => Promise.resolve(null)),
  openAgentResult: vi.fn(),
}))

vi.mock("@/lib/ipc/library", () => ({
  library: { openRetainedResult: backend.openRetainedResult },
}))
vi.mock("@/lib/ipc/results", () => ({
  results: {
    readResultPage: backend.readResultPage,
    forgetResult: backend.forgetResult,
  },
}))
vi.mock("@/features/workspace/result-requests", () => ({
  openAgentResult: backend.openAgentResult,
}))
// jsdom lays nothing out, so the virtualized grid would draw no row and ask
// no page. It is stood in for by a grid that reads its whole page once, as
// the real one does for a page of eight visible rows; its scrolling has its
// own stories.
vi.mock("@/components/oxyn/result-grid", async (importActual) => {
  const actual = await importActual<typeof ResultGridModule>()
  function ResultGrid({
    rowCount,
    fetchPage,
    "aria-label": label,
  }: {
    rowCount: number
    fetchPage: FetchPage
    "aria-label"?: string
  }) {
    const [first, setFirst] = React.useState<string | null>(null)
    React.useEffect(() => {
      void fetchPage(0, rowCount).then(
        (answer) => {
          if (answer.type === "page") setFirst(String(answer.rows[0]?.[0]))
        },
        () => undefined
      )
    }, [fetchPage, rowCount])
    return (
      <div role="grid" aria-label={label}>
        {first}
      </div>
    )
  }
  return { ...actual, ResultGrid }
})

const CONNECTION = "conn-agent"

function entry(result: string) {
  return {
    kind: "tool",
    key: `call-${result}`,
    call: 1,
    tool: "execute_query",
    command: "execute_query",
    statement: "SELECT * FROM invoices",
    connection: CONNECTION,
    environment: null,
    mutating: false,
    state: "completed",
    detail: null,
    errorClass: null,
    withheld: false,
    rows: 250,
    result,
    approval: null,
  } as unknown as ToolCallEntry & { result: string }
}

function opened(result: string, rows: number): RetainedResult {
  return {
    type: "open",
    result,
    columns: [{ name: "id", dataType: "Int64", nullable: false }],
    rows,
    complete: true,
    truncated: false,
  }
}

/** Rows numbered from 0, as the buffer holds them. */
function page(offset: number, limit: number, total: number): ResultWindow {
  return {
    type: "page",
    offset,
    rows: Array.from({ length: limit }, (_, row) => [String(offset + row)]),
    totalRows: total,
    complete: true,
  }
}

function view(result: string) {
  const client = new QueryClient({
    defaultOptions: { queries: { retry: false } },
  })
  const rendered = render(
    <QueryClientProvider client={client}>
      <ToolRows connection={CONNECTION} entry={entry(result)} />
    </QueryClientProvider>
  )
  return {
    rerender: (next: string) =>
      rendered.rerender(
        <QueryClientProvider client={client}>
          <ToolRows connection={CONNECTION} entry={entry(next)} />
        </QueryClientProvider>
      ),
  }
}

const reads = () => backend.readResultPage.mock.calls

async function click(name: string) {
  await act(async () => {
    fireEvent.click(screen.getByRole("button", { name }))
    await Promise.resolve()
  })
}

beforeEach(() => {
  backend.openRetainedResult.mockImplementation((_, result) =>
    Promise.resolve(opened(result, 250))
  )
  backend.readResultPage.mockImplementation((_, __, offset, limit) =>
    Promise.resolve(page(offset, limit, 250))
  )
})

afterEach(() => {
  cleanup()
  vi.clearAllMocks()
})

describe("paging the rows kept under an assistant answer", () => {
  it("opens the result once and reads each page from the buffer alone", async () => {
    view("res-1")
    expect(await screen.findByText("Rows 1–100 of 250")).toBeTruthy()
    await waitFor(() =>
      expect(reads()).toEqual([[CONNECTION, "res-1", 0, 100]])
    )

    await click("Next page")
    expect(await screen.findByText("Rows 101–200 of 250")).toBeTruthy()
    await waitFor(() =>
      expect(reads().at(-1)).toEqual([CONNECTION, "res-1", 100, 100])
    )

    await click("Next page")
    expect(await screen.findByText("Rows 201–250 of 250")).toBeTruthy()
    // The last page is short: nothing is asked past the rows retained.
    await waitFor(() =>
      expect(reads().at(-1)).toEqual([CONNECTION, "res-1", 200, 50])
    )

    await click("Previous page")
    expect(await screen.findByText("Rows 101–200 of 250")).toBeTruthy()
    await waitFor(() =>
      expect(reads().at(-1)).toEqual([CONNECTION, "res-1", 100, 100])
    )

    // Paging opened nothing again, released nothing, ran nothing.
    expect(backend.openRetainedResult).toHaveBeenCalledExactlyOnceWith(
      CONNECTION,
      "res-1"
    )
    expect(backend.forgetResult).not.toHaveBeenCalled()
    expect(backend.openAgentResult).not.toHaveBeenCalled()
    for (const [connection, result, offset, limit] of reads()) {
      expect([connection, result]).toEqual([CONNECTION, "res-1"])
      expect(offset + limit).toBeLessThanOrEqual(250)
    }
  })

  it("starts another result on its first page", async () => {
    const { rerender } = view("res-1")
    await screen.findByText("Rows 1–100 of 250")
    await click("Next page")
    await screen.findByText("Rows 101–200 of 250")

    backend.openRetainedResult.mockImplementation((_, result) =>
      Promise.resolve(opened(result, 1_000))
    )
    rerender("res-2")
    expect(await screen.findByText("Rows 1–100 of 1,000")).toBeTruthy()
    await waitFor(() =>
      expect(reads().at(-1)).toEqual([CONNECTION, "res-2", 0, 100])
    )
    expect(
      screen
        .getByRole("button", { name: "Previous page" })
        .getAttribute("aria-disabled")
    ).toBe("true")
    expect(backend.openRetainedResult).toHaveBeenCalledTimes(2)
    // The view stopped reading the first result: its reader is released.
    expect(backend.forgetResult).toHaveBeenCalledWith("res-1")
  })

  it("says a result released while paging is gone, and reruns nothing", async () => {
    backend.readResultPage.mockImplementation((_, __, offset, limit) =>
      Promise.resolve(
        offset < 100 ? page(offset, limit, 250) : { type: "expired" }
      )
    )
    view("res-1")
    await screen.findByText("Rows 1–100 of 250")
    await click("Next page")
    expect(
      await screen.findByText(/Result no longer available: Oxyn released/)
    ).toBeTruthy()
    expect(screen.queryByRole("grid")).toBeNull()
    expect(
      screen
        .getByRole("button", { name: "Next page" })
        .getAttribute("aria-disabled")
    ).toBe("true")
    expect(screen.queryByRole("button", { name: /Open all rows/ })).toBeNull()
    expect(backend.openRetainedResult).toHaveBeenCalledOnce()
    expect(backend.openAgentResult).not.toHaveBeenCalled()
  })
})
