import { afterEach, describe, expect, it, vi } from "vitest"

import {
  hostileAddress,
  hostileFacets,
} from "@/components/oxyn/metadata-fixtures"
import {
  consoleTextRequests,
  takeConsoleTextRequests,
} from "@/features/consoles/open-in-console"
import { setZoneHandle } from "@/lib/actions/context"
import { metadata } from "@/lib/ipc/metadata"
import type { CatalogNode } from "@/lib/ipc/types"
import { nameTransfer } from "./name-transfer"

const hostile: CatalogNode = {
  address: hostileAddress,
  name: 'users"; DROP TABLE audit; --',
  kind: "table",
  holdsRecords: true,
  system: false,
  comment: null,
  loaded: true,
  stale: false,
  children: [],
}

/** An editor under the pointer, as `SqlEditor` registers its zone. */
function editorUnderPointer(insertAt: (text: string) => void) {
  const zone = document.createElement("div")
  zone.dataset.actionZone = "editor"
  document.body.append(zone)
  setZoneHandle(zone, { insertAt })
  // jsdom lays nothing out: say what lies under the point.
  document.elementFromPoint = () => zone
  return zone
}

afterEach(() => {
  vi.restoreAllMocks()
  document.body.replaceChildren()
  takeConsoleTextRequests()
})

describe("a relation's name taken from the catalog", () => {
  it("lands in the editor as the backend quoted it, never rebuilt", async () => {
    vi.spyOn(metadata, "relationFacets").mockResolvedValue(hostileFacets)
    const insertAt = vi.fn()
    editorUnderPointer(insertAt)
    nameTransfer("connection-1", vi.fn()).onDrop(hostile, { x: 40, y: 12 })
    await vi.waitFor(() =>
      expect(insertAt).toHaveBeenCalledWith(
        '"public"."users""; DROP TABLE audit; --"',
        { x: 40, y: 12 }
      )
    )
    expect(metadata.relationFacets).toHaveBeenCalledWith(
      "connection-1",
      hostileAddress
    )
  })

  it("asks for nothing when dropped anywhere but an editor", () => {
    const facets = vi.spyOn(metadata, "relationFacets")
    document.elementFromPoint = () => document.body
    nameTransfer("connection-1", vi.fn()).onDrop(hostile, { x: 1, y: 1 })
    expect(facets).not.toHaveBeenCalled()
  })

  it("goes to the active console from the keyboard, unrun", async () => {
    vi.spyOn(metadata, "relationFacets").mockResolvedValue(hostileFacets)
    nameTransfer("connection-1", vi.fn()).onInsert(hostile)
    await vi.waitFor(() => expect(consoleTextRequests.state).toHaveLength(1))
    expect(consoleTextRequests.state[0]).toMatchObject({
      sql: '"public"."users""; DROP TABLE audit; --"',
      newConsole: false,
    })
  })

  it("reports a failed lookup instead of inserting a guess", async () => {
    const failure = new Error("catalog closed")
    vi.spyOn(metadata, "relationFacets").mockRejectedValue(failure)
    const insertAt = vi.fn()
    editorUnderPointer(insertAt)
    const onProblem = vi.fn()
    nameTransfer("connection-1", onProblem).onDrop(hostile, { x: 4, y: 4 })
    await vi.waitFor(() => expect(onProblem).toHaveBeenCalledWith(failure))
    expect(insertAt).not.toHaveBeenCalled()
  })
})
