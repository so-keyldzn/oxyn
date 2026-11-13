import { afterEach, beforeEach, expect, it, vi } from "vitest"

import { metadataReads } from "./read-requests"
import type { CommandOutcome } from "@/lib/ipc/types"

const ipc = vi.hoisted(() => ({
  next: 0,
  cancel: vi.fn(() => Promise.resolve()),
  calls: [] as Array<{
    id: string
    session: string
    resolve: (value: CommandOutcome) => void
  }>,
}))
vi.mock("@/lib/ipc/client", () => ({
  backend: { cancel: ipc.cancel },
  newCommandId: () => `metadata-${++ipc.next}`,
}))
vi.mock("@/lib/ipc/metadata", () => ({
  metadata: {
    refreshCatalog: (id: string, _connection: string, session: string) =>
      new Promise<CommandOutcome>((resolve) =>
        ipc.calls.push({ id, session, resolve })
      ),
    refreshRelationFacet: (id: string, _connection: string, session: string) =>
      new Promise<CommandOutcome>((resolve) =>
        ipc.calls.push({ id, session, resolve })
      ),
  },
}))
const address = { catalog: null, namespace: "temp", relation: "records" }
const flush = async () => {
  for (let i = 0; i < 8; i++) await Promise.resolve()
}

beforeEach(() => {
  ipc.next = 0
  ipc.calls.length = 0
  ipc.cancel.mockClear()
})
afterEach(async () => {
  for (let i = 0; i < 4; i++) {
    for (const call of ipc.calls) call.resolve({ type: "done" })
    await flush()
  }
})

it("shares an active facet but reads again after an invalidation without cancelling its owner", async () => {
  const first = metadataReads.facet("c", "s", address, "detail")
  expect(metadataReads.facet("c", "s", address, "detail")).toBe(first)
  const fresh = metadataReads.facet("c", "s", address, "detail", true)
  expect(fresh.id).not.toBe(first.id)
  expect(metadataReads.facet("c", "s", address, "detail", true)).toBe(fresh)
  expect(ipc.calls).toHaveLength(1)
  expect(ipc.cancel).not.toHaveBeenCalled()
  ipc.calls[0]?.resolve({ type: "done" })
  await first.outcome
  await flush()
  expect(ipc.calls).toHaveLength(2)
  expect(ipc.calls[1]?.id).toBe(fresh.id)
  ipc.calls[1]?.resolve({ type: "done" })
  await fresh.outcome
})

it("an explicit stop also prevents a queued replacement from starting", async () => {
  const first = metadataReads.facet("c", "s", address, "detail")
  const replacement = metadataReads.facet("c", "s", address, "detail", true)
  await metadataReads.cancelFacet("c", "s", address, "detail")
  expect(ipc.cancel).toHaveBeenCalledExactlyOnceWith(first.id)
  ipc.calls[0]?.resolve({ type: "cancelled" })
  expect(await replacement.outcome).toEqual({ type: "cancelled" })
  expect(ipc.calls).toHaveLength(1)
})

it("cancels expanded catalog levels only on the selected source", async () => {
  const root = metadataReads.catalog("c", "s", null)
  const level = metadataReads.catalog("c", "s", { ...address, relation: null })
  const other = metadataReads.catalog("c", "other", null)
  await metadataReads.cancelCatalog("c", "s")
  expect(ipc.cancel.mock.calls).toEqual([[root.id], [level.id]])
  expect(ipc.cancel).not.toHaveBeenCalledWith(other.id)
})
