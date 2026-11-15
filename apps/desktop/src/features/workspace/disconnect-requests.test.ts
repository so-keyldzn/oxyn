import { afterEach, describe, expect, it } from "vitest"
import {
  disconnectRequests,
  finishWorkspaceDisconnect,
  reviewWorkspaceDisconnect,
  takeWorkspaceDisconnect,
} from "./disconnect-requests"

afterEach(() => {
  for (const id of disconnectRequests.state)
    finishWorkspaceDisconnect(id, false)
})
describe("owner-reviewed disconnects", () => {
  it("retains the first owner until its actual cleanup settles", async () => {
    const a = reviewWorkspaceDisconnect("a")
    const b = reviewWorkspaceDisconnect("b")
    expect(takeWorkspaceDisconnect("b")).toBe(false)
    expect(takeWorkspaceDisconnect("a")).toBe(true)
    expect(takeWorkspaceDisconnect("a")).toBe(false)
    expect(disconnectRequests.state).toEqual(["a", "b"])
    finishWorkspaceDisconnect("a", true)
    expect(await a).toBe(true)
    expect(takeWorkspaceDisconnect("b")).toBe(true)
    finishWorkspaceDisconnect("b", false)
    expect(await b).toBe(false)
  })
  it("coalesces requests and reports cancellation to every waiter", async () => {
    const first = reviewWorkspaceDisconnect("a")
    const repeated = reviewWorkspaceDisconnect("a")
    expect(disconnectRequests.state).toEqual(["a"])
    finishWorkspaceDisconnect("a", false)
    expect(await Promise.all([first, repeated])).toEqual([false, false])
    finishWorkspaceDisconnect("a", true)
    expect(disconnectRequests.state).toEqual([])
  })
})
