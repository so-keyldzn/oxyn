import * as React from "react"
import { QueryClient, QueryClientProvider } from "@tanstack/react-query"
import { act, renderHook, waitFor } from "@testing-library/react"
import { describe, expect, it, vi } from "vitest"

import { useResultExport } from "./export-menu"
import type { ExportFormatChoice } from "@/lib/ipc/results"

const calls = vi.hoisted(() => ({
  exportResult: vi.fn(),
  toast: vi.fn(),
}))

vi.mock("@/lib/ipc/client", () => ({
  BackendError: class extends Error {},
  newCommandId: () => "export-1",
  backend: { cancel: vi.fn() },
}))
vi.mock("@/lib/ipc/results", () => ({
  results: {
    exportFormats: () => Promise.resolve([]),
    exportResult: calls.exportResult,
  },
}))
vi.mock("@/components/ui/toast", () => ({ toast: { add: calls.toast } }))

const csv: ExportFormatChoice = {
  format: "csv",
  label: "CSV",
  extension: "csv",
  supported: true,
}

function withClient(client: QueryClient) {
  return function Wrapper({ children }: { children: React.ReactNode }) {
    return <QueryClientProvider client={client}>{children}</QueryClientProvider>
  }
}

describe("result export", () => {
  it("says a cancelled export left the existing file unchanged", async () => {
    calls.exportResult.mockResolvedValueOnce({ type: "cancelled" })
    const client = new QueryClient({
      defaultOptions: { queries: { retry: false } },
    })
    const { result } = renderHook(
      () =>
        useResultExport({
          connection: "connection-1",
          result: "result-1",
        }),
      { wrapper: withClient(client) }
    )

    act(() => result.current.run(csv))

    await waitFor(() =>
      expect(calls.toast).toHaveBeenCalledWith({
        title: "Export cancelled",
        description:
          "No file was written; an existing file was left unchanged.",
        type: "warning",
      })
    )
  })
})
