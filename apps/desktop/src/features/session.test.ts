import { beforeEach, describe, expect, it } from "vitest"

import {
  MAX_RETAINED_WORKSPACES,
  canOpenWorkspace,
  closeConnection,
  connectionReleased,
  openConnection,
  refreshConnection,
  restoreWorkingCopies,
  session,
  showConnection,
  takeRestoredWorkingCopies,
} from "./session"
import type { DocumentEntry } from "@/lib/ipc/library"
import type { OpenConnection } from "@/lib/ipc/types"

const entry = (id: string) => ({ id }) as unknown as DocumentEntry

const opened = (connection: string) =>
  ({
    connection,
    session: `${connection}-s`,
    privacyTier: "metadata",
  }) as OpenConnection

describe("retained workspaces", () => {
  beforeEach(() =>
    session.setState((state) => ({ ...state, open: null, workspaces: [] }))
  )

  it("keeps A when B opens, and shows A again without reopening it", () => {
    const a = opened("a")
    openConnection(a)
    openConnection(opened("b"))
    expect(session.state.workspaces.map((open) => open.connection)).toEqual([
      "a",
      "b",
    ])
    expect(showConnection("a")).toBe(true)
    // The very session A opened with: nothing reconnected.
    expect(session.state.open).toBe(a)
    expect(showConnection("c")).toBe(false)
  })

  it("forgets only the connection explicitly disconnected", () => {
    openConnection(opened("a"))
    openConnection(opened("b"))
    closeConnection("a")
    expect(session.state.workspaces.map((open) => open.connection)).toEqual([
      "b",
    ])
    expect(session.state.open?.connection).toBe("b")
    closeConnection()
    expect(session.state.open).toBeNull()
    expect(session.state.workspaces).toEqual([])
  })

  it("keeps a disconnected connection for a reload until its sessions close", () => {
    openConnection(opened("a"))
    openConnection(opened("b"))
    closeConnection("a")
    // Drafts are still being written: a reload now must close A too.
    expect(sessionStorage.getItem("oxyn.open-connections")).toBe('["b","a"]')
    connectionReleased("a")
    expect(sessionStorage.getItem("oxyn.open-connections")).toBe('["b"]')
  })

  it("refuses another connection past the bound, never one already open", () => {
    for (let index = 0; index < MAX_RETAINED_WORKSPACES; index += 1)
      openConnection(opened(`c${index}`))
    expect(canOpenWorkspace()).toBe(false)
    expect(canOpenWorkspace("c0")).toBe(true)
    closeConnection("c3")
    expect(canOpenWorkspace()).toBe(true)
  })

  it("hands a hidden workspace its new tier without showing it", () => {
    openConnection(opened("a"))
    openConnection(opened("b"))
    refreshConnection({ ...opened("a"), privacyTier: "local" })
    expect(session.state.open?.connection).toBe("b")
    expect(session.state.workspaces[0]?.privacyTier).toBe("local")
  })
})

describe("restored working copies", () => {
  it("add up across selections instead of replacing those still waiting", () => {
    restoreWorkingCopies([entry("a"), entry("b")])
    restoreWorkingCopies([entry("b"), entry("c")])
    expect(session.state.restored.map((copy) => copy.id)).toEqual([
      "a",
      "b",
      "c",
    ])
    expect(takeRestoredWorkingCopies().map((copy) => copy.id)).toEqual([
      "a",
      "b",
      "c",
    ])
    expect(session.state.restored).toEqual([])
  })
})
