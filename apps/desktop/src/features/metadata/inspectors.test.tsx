import { QueryClientProvider } from "@tanstack/react-query"
import {
  act,
  cleanup,
  fireEvent,
  render,
  screen,
  waitFor,
} from "@testing-library/react"
import { afterEach, beforeAll, describe, expect, it, vi } from "vitest"

import { invoiceColumns } from "@/components/oxyn/fixtures"
import { selectRow } from "@/features/metadata/inspection"
import { ValueInspectorPanel } from "@/features/metadata/inspectors"
import { preferencesStore } from "@/features/settings/preferences"
import { useResultFormatRefresh } from "@/features/settings/use-result-format-refresh"
import { BackendError } from "@/lib/ipc/client"
import { createQueryClient } from "@/lib/query-client"
import type { OpenConnection } from "@/lib/ipc/types"

const read = vi.hoisted(() => vi.fn())
vi.mock("@/lib/ipc/results", () => ({ results: { readResultPage: read } }))

const open: OpenConnection = {
  connection: "connection",
  session: "session",
  name: "Test",
  driver: "sqlite",
  environment: "local",
  privacyTier: "local",
  readOnly: true,
  capabilities: [],
  console: {
    session: "console",
    capabilities: [],
    readOnly: true,
    transactionState: "idle",
  },
}

beforeAll(() => {
  Element.prototype.scrollIntoView = vi.fn()
})
afterEach(() => {
  cleanup()
  selectRow(null)
  vi.resetAllMocks()
})

function showInspector() {
  const client = createQueryClient()
  selectRow({
    source: "test",
    connection: open.connection,
    connectionName: open.name,
    result: "result",
    row: 0,
    column: 0,
    columns: invoiceColumns,
  })
  function Panel() {
    useResultFormatRefresh(client)
    return <ValueInspectorPanel open={open} />
  }
  return render(
    <QueryClientProvider client={client}>
      <Panel />
    </QueryClientProvider>
  )
}

describe("record inspector page states", () => {
  it.each([false, true])(
    "shows a failed read and only offers an allowed retry: %s",
    async (retryable) => {
      read.mockRejectedValueOnce(
        new BackendError({ message: "Page read failed", retryable })
      )
      read.mockResolvedValue({ type: "page", rows: [["Recovered row"]] })
      const { container } = showInspector()
      expect((await screen.findByRole("alert")).textContent).toContain(
        "Page read failed"
      )
      expect(container.querySelector('[data-slot="skeleton"]')).toBeNull()
      expect(read).toHaveBeenCalledTimes(1)
      if (retryable) {
        fireEvent.click(screen.getByRole("button", { name: "Try again" }))
        await screen.findByText("Recovered row")
        expect(read).toHaveBeenLastCalledWith(open.connection, "result", 0, 1)
      } else
        expect(screen.queryByRole("button", { name: "Try again" })).toBeNull()
    }
  )

  it.each([{ type: "expired" }, { type: "page", rows: [] }])(
    "shows an unavailable row instead of waiting forever: %o",
    async (answer) => {
      read.mockResolvedValue(answer)
      const { container } = showInspector()
      await screen.findByText("Row no longer available")
      expect(container.querySelector('[data-slot="skeleton"]')).toBeNull()
      expect(read).toHaveBeenCalledTimes(1)
    }
  )

  it("refreshes the inspector's formatted cells with the grid", async () => {
    read.mockResolvedValueOnce({ type: "page", rows: [["4823917"]] })
    read.mockResolvedValue({ type: "page", rows: [["4,823,917"]] })
    showInspector()
    await screen.findByText("4823917")
    act(() =>
      preferencesStore.setState((state) => ({
        ...state,
        formatRevision: state.formatRevision + 1,
      }))
    )
    await screen.findByText("4,823,917")
    await waitFor(() => expect(read).toHaveBeenCalledTimes(2))
  })
})
