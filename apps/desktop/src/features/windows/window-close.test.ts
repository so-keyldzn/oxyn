import { afterEach, describe, expect, it, vi } from "vitest"

import {
  cancelWindowClose,
  closeWindow,
  onCloseRequested,
  registerWorkspaceConsoles,
  windowClose,
  windowCloseRows,
} from "./window-close"
import type { WorkspaceConsoles } from "./window-close"

const backend = vi.hoisted(() => ({
  acknowledged: 0,
  cancelled: 0,
  confirmed: 0,
  flushed: 0,
  /** What happened, in order: the backend must hear of the close last. */
  log: [] as Array<string>,
  /** The drafts' flush; a test holds it to cancel meanwhile. */
  flush: () => Promise.resolve(true),
}))

vi.mock("@/lib/ipc/recovery", () => ({
  recovery: {
    shutdownAcknowledged: () => {
      backend.acknowledged += 1
      return Promise.resolve()
    },
    cancelExit: () => {
      backend.cancelled += 1
      return Promise.resolve()
    },
  },
}))

vi.mock("@/lib/ipc/windows", () => ({
  windows: {
    confirmClose: () => {
      backend.confirmed += 1
      backend.log.push("confirm")
      return Promise.resolve()
    },
  },
}))

vi.mock("@/features/consoles/draft-registry", () => ({
  flushAllDrafts: () => {
    backend.flushed += 1
    backend.log.push("flush")
    return backend.flush()
  },
}))

function workspace(
  connection: string,
  costs: ReturnType<WorkspaceConsoles["costs"]>
): WorkspaceConsoles {
  return {
    connection,
    costs: () => costs,
    closeAll: () => {
      backend.log.push(`close ${connection}`)
      return Promise.resolve()
    },
  }
}

/** Holds the drafts' flush until the test answers it. */
function holdFlush() {
  let finish = (_flushed: boolean) => {}
  backend.flush = () =>
    new Promise<boolean>((resolve) => {
      finish = resolve
    })
  return (flushed: boolean) => finish(flushed)
}

const UNSAVED = {
  title: "report.sql",
  unsaved: true,
  conflict: false,
  running: false,
}

describe("closing a window that is not the last", () => {
  const unregister: Array<() => void> = []

  afterEach(() => {
    unregister.splice(0).forEach((undo) => undo())
    windowClose.setState(() => null)
    Object.assign(backend, {
      acknowledged: 0,
      cancelled: 0,
      confirmed: 0,
      flushed: 0,
      log: [],
      flush: () => Promise.resolve(true),
    })
  })

  it("lists the consoles that would lose work, across every workspace", () => {
    const held = new Map([
      ["a", workspace("billing", [UNSAVED])],
      ["b", workspace("analytics", [])],
    ])
    const asked = windowCloseRows(held)
    expect(asked.connections).toEqual(["billing", "analytics"])
    expect(asked.rows).toEqual([
      { ...UNSAVED, key: "a:0", connection: "billing" },
    ])
  })

  it("acknowledges at once, then asks when a console would lose work", () => {
    unregister.push(
      registerWorkspaceConsoles("a", workspace("billing", [UNSAVED]))
    )
    onCloseRequested()
    expect(backend.acknowledged).toBe(1)
    expect(windowClose.state?.rows).toHaveLength(1)
    expect(backend.confirmed).toBe(0)
  })

  it("closes at once when nothing would be lost, consoles before the window", async () => {
    unregister.push(registerWorkspaceConsoles("a", workspace("billing", [])))
    unregister.push(registerWorkspaceConsoles("b", workspace("analytics", [])))
    onCloseRequested()
    await vi.waitFor(() => expect(backend.confirmed).toBe(1))
    expect(backend.log).toEqual([
      "flush",
      "close billing",
      "close analytics",
      "confirm",
    ])
  })

  it("closes everything once confirmed", async () => {
    unregister.push(
      registerWorkspaceConsoles("a", workspace("billing", [UNSAVED]))
    )
    onCloseRequested()
    await closeWindow()
    expect(backend.log).toEqual(["flush", "close billing", "confirm"])
  })

  it("keeps the window and tells the backend on Cancel", () => {
    unregister.push(
      registerWorkspaceConsoles("a", workspace("billing", [UNSAVED]))
    )
    onCloseRequested()
    cancelWindowClose()
    expect(windowClose.state).toBeNull()
    expect(backend.cancelled).toBe(1)
    expect(backend.log).toEqual([])
  })

  it("closes no console after Cancel while the drafts flush", async () => {
    unregister.push(
      registerWorkspaceConsoles("a", workspace("billing", [UNSAVED]))
    )
    const finish = holdFlush()
    onCloseRequested()
    const pending = closeWindow()
    cancelWindowClose()
    finish(true)
    await pending
    expect(backend.cancelled).toBe(1)
    expect(backend.log).toEqual(["flush"])
    expect(backend.confirmed).toBe(0)
  })

  it("starts no further workspace once Cancel stops the closing one", async () => {
    let finish = () => {}
    let signal: AbortSignal | undefined
    unregister.push(
      registerWorkspaceConsoles("a", {
        ...workspace("billing", [UNSAVED]),
        closeAll: (given) => {
          signal = given
          backend.log.push("close billing")
          return new Promise((resolve) => {
            finish = resolve
          })
        },
      })
    )
    unregister.push(registerWorkspaceConsoles("b", workspace("analytics", [])))
    onCloseRequested()
    const pending = closeWindow()
    await vi.waitFor(() => expect(backend.log).toContain("close billing"))
    cancelWindowClose()
    expect(signal?.aborted).toBe(true)
    finish()
    await pending
    expect(backend.log).toEqual(["flush", "close billing"])
    expect(backend.confirmed).toBe(0)
  })

  it("lets a cancelled close touch nothing of the next one", async () => {
    unregister.push(
      registerWorkspaceConsoles("a", workspace("billing", [UNSAVED]))
    )
    const finishOld = holdFlush()
    onCloseRequested()
    const old = closeWindow()
    cancelWindowClose()

    const finishNew = holdFlush()
    onCloseRequested()
    const next = closeWindow()
    finishOld(true)
    await old
    expect(backend.log).toEqual(["flush", "flush"])

    finishNew(true)
    await next
    expect(backend.log).toEqual(["flush", "flush", "close billing", "confirm"])
  })

  it("forgets a workspace that unmounted", () => {
    const undo = registerWorkspaceConsoles("a", workspace("billing", [UNSAVED]))
    undo()
    expect(windowCloseRows().rows).toEqual([])
  })
})
