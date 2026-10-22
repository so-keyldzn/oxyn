import { describe, expect, it } from "vitest"

import { historyCopyFailure } from "./workspace-screen"
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
})
