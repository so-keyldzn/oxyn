import { describe, expect, it } from "vitest"

import {
  restoreWorkingCopies,
  session,
  takeRestoredWorkingCopies,
} from "./session"
import type { DocumentEntry } from "@/lib/ipc/library"

const entry = (id: string) => ({ id }) as unknown as DocumentEntry

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
