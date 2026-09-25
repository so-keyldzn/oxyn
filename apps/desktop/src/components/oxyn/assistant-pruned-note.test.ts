import { describe, expect, it } from "vitest"

import { prunedSentence } from "./assistant-pruned-note"
import { PrunedHistory } from "@/lib/ipc/ai"

describe("the note on the launch prune", () => {
  it("says the rule with the backend's numbers, not copies of them", () => {
    expect(
      prunedSentence({
        conversations: 2,
        maxConversations: 50,
        maxAgeDays: 7,
        maxBytes: 8 * 1024 * 1024,
      })
    ).toBe(
      "Oxyn removed 2 conversations when it started. It keeps the 50 most recent, within 8 MiB, and removes those idle for 7 days."
    )
  })

  it("reads what the backend sends, `null` age bound included", () => {
    expect(
      PrunedHistory.parse({
        conversations: 1,
        maxConversations: 200,
        maxAgeDays: null,
        maxBytes: 33554432,
      }).maxAgeDays
    ).toBeNull()
    // Nothing removed is `null` on the wire, never a zero count.
    expect(() =>
      PrunedHistory.parse({
        conversations: 0,
        maxConversations: 200,
        maxAgeDays: 90,
        maxBytes: 33554432,
      })
    ).toThrow()
  })
})
