import { afterEach, describe, expect, it, vi } from "vitest"

import type { OpenConnection } from "@/lib/ipc/types"

const calls = vi.hoisted(() => ({
  disconnect: vi.fn(() => Promise.resolve()),
  close: vi.fn(() => Promise.resolve()),
  closeConversation: vi.fn(() => Promise.resolve()),
}))

vi.mock("@/lib/ipc/client", () => ({
  backend: { disconnect: calls.disconnect },
}))
vi.mock("@/lib/ipc/consoles", () => ({ consoles: { close: calls.close } }))
vi.mock("@/features/assistant/conversation-store", () => ({
  closeConversation: calls.closeConversation,
}))

const { release } = await import("./release-workspace")

const opened = (connection: string, session: string) =>
  ({ connection, session }) as OpenConnection

afterEach(() => vi.clearAllMocks())

describe("releasing a workspace", () => {
  it("closes the conversations of a connection it disconnects", async () => {
    // Without it, the external agent kept for the connection outlives it,
    // with the tools and the tier it was launched under (I-04).
    await release(opened("c1", "s1"), opened("c2", "s2"), null)
    expect(calls.disconnect).toHaveBeenCalledWith("c1")
    expect(calls.closeConversation).toHaveBeenCalledWith("c1")
  })

  it("keeps the conversations when the same connection is reopened", async () => {
    await release(opened("c1", "s1"), opened("c1", "s2"), null)
    expect(calls.close).toHaveBeenCalledWith("c1", "s1")
    expect(calls.disconnect).not.toHaveBeenCalled()
    expect(calls.closeConversation).not.toHaveBeenCalled()
  })
})
