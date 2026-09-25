import { describe, expect, it } from "vitest"

import { consoleSeed } from "./use-consoles"

describe("the seed of a new console", () => {
  it("is an empty console, on the server's default context", () => {
    expect(consoleSeed({}, "doc-1", "Console 2")).toEqual({
      document: "doc-1",
      revision: 0,
      title: "Console 2",
      text: "",
      savedTitle: null,
      savedText: null,
      hasSavedCopy: false,
      fromAgent: false,
      parameters: [],
      needsValues: false,
      context: null,
    })
  })

  it("carries the schema of « New console on this schema », and no text", () => {
    const place = { catalog: "billing", namespace: "reporting" }
    const seed = consoleSeed({ context: place }, "doc-1", "Console 3")
    expect(seed.context).toEqual(place)
    // The context changes what the session resolves, never the text.
    expect(seed.text).toBe("")
  })
})
