import { describe, expect, it, vi } from "vitest"

import { historyCopyFailure, openHistoryCopy } from "./workspace-screen"
import { BackendError } from "@/lib/ipc/client"

describe("opening a history copy", () => {
  it("names a rejected backend read and its retryability", () => {
    expect(
      historyCopyFailure(
        new BackendError({ message: "history is busy", retryable: true })
      )
    ).toEqual({
      title: "History copy not opened",
      description: "history is busy The copy can be tried again as is.",
      type: "error",
    })
  })

  it("reports a rejected history read without opening a console", async () => {
    const error = new BackendError({
      message: "history is busy",
      retryable: true,
    })
    const readHistoryEntry = vi.fn(() => Promise.reject(error))
    const open = vi.fn()
    const report = vi.fn()

    await openHistoryCopy({ id: 42 }, readHistoryEntry, open, report)

    expect(readHistoryEntry).toHaveBeenCalledWith(42)
    expect(open).not.toHaveBeenCalled()
    expect(report).toHaveBeenCalledWith(historyCopyFailure(error))
  })
})
