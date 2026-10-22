import { isRedirect } from "@tanstack/react-router"
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest"

import { session } from "@/features/session"
import { BackendError } from "@/lib/ipc/client"
import type { DocumentEntry } from "@/lib/ipc/library"
import { Route } from "./index"

const status = vi.hoisted(() => vi.fn())
const restoredConsoles = vi.hoisted(() => vi.fn())
const takeHandoff = vi.hoisted(() => vi.fn())
const toastAdd = vi.hoisted(() => vi.fn())

vi.mock("@/lib/ipc/recovery", () => ({ recovery: { status } }))
vi.mock("@/lib/ipc/windows", () => ({
  windows: { restoredConsoles, takeHandoff },
}))
vi.mock("@/components/ui/toast", () => ({ toast: { add: toastAdd } }))
// The screen itself is not under test, only what precedes it.
vi.mock("@/features/connections/connection-screen", () => ({
  ConnectionScreen: () => null,
}))

const copy: DocumentEntry = {
  id: "018f0000-0000-7000-8000-00000000c0b1",
  title: "unpaid invoices.sql",
  connection: null,
  connectionName: null,
  updatedAt: "2026-10-21T08:00:00Z",
  isSaved: false,
  isOpen: true,
  hasChanges: true,
  fromAgent: false,
}

type BeforeLoadContext = NonNullable<
  Parameters<NonNullable<typeof Route.options.beforeLoad>>[0]
>

/** What `beforeLoad` decided: a redirection, or staying on the list. */
async function launch() {
  const beforeLoad = Route.options.beforeLoad
  if (!beforeLoad) throw new Error("the index route lost its beforeLoad")
  try {
    // It reads nothing from its context: the launch state is the session's.
    await beforeLoad({} as BeforeLoadContext)
    return null
  } catch (thrown) {
    if (!isRedirect(thrown)) throw thrown
    return { to: thrown.options.to, search: thrown.options.search }
  }
}

const toRecovery = { to: "/recovery", search: { startup: true } }

beforeEach(() => {
  session.setState((state) => ({
    ...state,
    restored: [],
    launchCopies: null,
    recoveryOffered: false,
  }))
  takeHandoff.mockResolvedValue(null)
})

afterEach(() => {
  vi.clearAllMocks()
})

describe("the index route at launch", () => {
  it("restores the consoles of an ordinary close without a question", async () => {
    status.mockResolvedValue({ abnormal: false, unresolvedWrite: false })
    restoredConsoles.mockResolvedValue([copy])
    await expect(launch()).resolves.toBeNull()
    expect(session.state.restored).toEqual([copy])
    expect(toastAdd).not.toHaveBeenCalled()
  })

  it("offers the copies after an abnormal stop", async () => {
    status.mockResolvedValue({ abnormal: true, unresolvedWrite: false })
    restoredConsoles.mockResolvedValue([copy])
    await expect(launch()).resolves.toEqual(toRecovery)
    expect(session.state.launchCopies).toEqual([copy])
    expect(session.state.restored).toEqual([])
  })

  it("shows the screen after an abnormal stop that left no copy", async () => {
    // A write interrupted with no console open: the warning is all there is.
    status.mockResolvedValue({ abnormal: true, unresolvedWrite: true })
    restoredConsoles.mockResolvedValue([])
    await expect(launch()).resolves.toEqual(toRecovery)
    expect(session.state.launchCopies).toEqual([])
  })

  it("never reads a failed status as a clean shutdown", async () => {
    status.mockRejectedValue(
      new BackendError({ message: "status unavailable", retryable: false })
    )
    restoredConsoles.mockResolvedValue([])
    await expect(launch()).resolves.toEqual(toRecovery)
    expect(toastAdd).toHaveBeenCalledWith(
      expect.objectContaining({
        title: "How Oxyn last closed could not be read",
        description: "status unavailable",
      })
    )
  })

  it("reports unread consoles and lets the screen list every open copy", async () => {
    status.mockResolvedValue({ abnormal: false, unresolvedWrite: false })
    restoredConsoles.mockRejectedValue(new Error("restored_consoles failed"))
    await expect(launch()).resolves.toEqual(toRecovery)
    expect(toastAdd).toHaveBeenCalledWith(
      expect.objectContaining({
        title: "This window's consoles could not be read",
        description: "Error: restored_consoles failed",
      })
    )
    // Not this window's list, which was not read: the library's open copies.
    expect(session.state.launchCopies).toBeNull()
    expect(session.state.restored).toEqual([])
  })

  it("asks only once per launch", async () => {
    status.mockResolvedValue({ abnormal: true, unresolvedWrite: false })
    restoredConsoles.mockResolvedValue([])
    await expect(launch()).resolves.toEqual(toRecovery)
    await expect(launch()).resolves.toBeNull()
    expect(status).toHaveBeenCalledTimes(1)
  })
})
