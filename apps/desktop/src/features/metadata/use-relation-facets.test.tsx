import { StrictMode, useEffect } from "react"
import type { ReactNode } from "react"
import { QueryClient, QueryClientProvider } from "@tanstack/react-query"
import { act, cleanup, renderHook, waitFor } from "@testing-library/react"
import { afterEach, expect, it, vi } from "vitest"

import { facetsKey, useRelationFacets } from "./use-relation-facets"
import type { RelationFacets } from "@/lib/ipc/metadata"
import type { CommandOutcome, OpenConnection } from "@/lib/ipc/types"

const ipc = vi.hoisted(() => ({
  cancel: vi.fn(() => Promise.resolve()),
  refresh: vi.fn(),
  resolve: (_value: CommandOutcome) => {},
}))
const never = { freshness: { state: "never" }, value: null }
const facets = {
  detail: never,
  constraints: never,
  incomingKeys: never,
  definition: never,
} as RelationFacets
vi.mock("@/lib/ipc/client", () => ({
  BackendError: class extends Error {
    retryable = false
    missingModule = null
  },
  backend: { cancel: ipc.cancel, decide: vi.fn() },
  newCommandId: () => "facet-command",
}))
vi.mock("@/lib/ipc/metadata", () => ({
  metadata: {
    relationFacets: () => Promise.resolve(facets),
    refreshRelationFacet: (...args: unknown[]) => {
      ipc.refresh(...args)
      return new Promise<CommandOutcome>((resolve) => {
        ipc.resolve = resolve
      })
    },
  },
}))
vi.mock("./refresh-signals", () => ({ useRefreshSignal: () => undefined }))

afterEach(async () => {
  cleanup()
  await act(async () => {
    ipc.resolve({ type: "done" })
  })
})

it("keeps a facet read through StrictMode and reattachment, with an explicit Stop", async () => {
  const open = {
    connection: "facet-connection",
    session: "facet-owner",
  } as OpenConnection
  const address = { catalog: null, namespace: "temp", relation: "records" }
  const client = new QueryClient({
    defaultOptions: { queries: { retry: false, staleTime: Infinity } },
  })
  client.setQueryData(facetsKey(open.connection, address, open.session), facets)
  const show = () =>
    renderHook(
      () => {
        const result = useRelationFacets(open, address)
        useEffect(() => {
          result.ensure("detail")
        }, [result.ensure])
        return result
      },
      {
        wrapper: ({ children }: { children: ReactNode }) => (
          <StrictMode>
            <QueryClientProvider client={client}>
              {children}
            </QueryClientProvider>
          </StrictMode>
        ),
      }
    )
  const first = show()
  await waitFor(() => expect(ipc.refresh).toHaveBeenCalledTimes(1))
  expect(ipc.cancel).not.toHaveBeenCalled()
  first.unmount()
  const second = show()
  await waitFor(() =>
    expect(second.result.current.loads.detail.status).toBe("loading")
  )
  expect(ipc.refresh).toHaveBeenCalledTimes(1)
  expect(ipc.cancel).not.toHaveBeenCalled()
  act(() => second.result.current.cancel("detail"))
  expect(ipc.cancel).toHaveBeenCalledExactlyOnceWith("facet-command")
  await act(async () => {
    ipc.resolve({ type: "cancelled" })
  })
  expect(second.result.current.loads.detail.status).toBe("cancelled")
})
