import * as React from "react"
import { QueryClientProvider } from "@tanstack/react-query"
import { act, cleanup, renderHook } from "@testing-library/react"
import { afterEach, expect, it, vi } from "vitest"

import { useConsoleDocument } from "./use-console-document"
import { createQueryClient } from "@/lib/query-client"

const closeDocument = vi.hoisted(() => vi.fn())
vi.mock("@/lib/ipc/library", () => ({ library: { closeDocument } }))
vi.mock("@/lib/ipc/client", () => ({
  backend: { cancel: vi.fn() },
  newCommandId: () => "close-command",
}))
afterEach(() => {
  cleanup()
  vi.resetAllMocks()
})

function document() {
  const client = createQueryClient()
  return renderHook(
    () =>
      useConsoleDocument({
        connection: "connection",
        seed: {
          document: "document",
          revision: 1,
          title: "Query",
          text: "SELECT 1",
          savedTitle: "Query",
          savedText: "SELECT 1",
          hasSavedCopy: true,
          fromAgent: false,
          parameters: [],
          needsValues: false,
          context: null,
        },
      }),
    {
      wrapper: ({ children }: { children: React.ReactNode }) => (
        <QueryClientProvider client={client}>{children}</QueryClientProvider>
      ),
    }
  )
}

it("keeps a committed document read-only until the closing batch removes its tab", async () => {
  closeDocument.mockResolvedValue({ type: "closed" })
  const { result } = document()
  await act(async () => {
    expect(await result.current.close(true)).toBe(true)
  })
  expect(result.current.closing).toBe(true)
  act(() =>
    result.current.change({
      text: "An edit after Cancel must not be lost with a closed document",
    })
  )
  expect(result.current.text).toBe("SELECT 1")
})

it("unlocks the document when its close failed", async () => {
  closeDocument.mockRejectedValue(new Error("The workspace is busy"))
  const { result } = document()
  await act(async () => {
    expect(await result.current.close(true)).toBe(false)
  })
  expect(result.current.closing).toBe(false)
  act(() => result.current.change({ text: "SELECT 2" }))
  expect(result.current.text).toBe("SELECT 2")
})
