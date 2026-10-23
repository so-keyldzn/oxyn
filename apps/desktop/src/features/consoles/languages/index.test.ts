import { describe, expect, it } from "vitest"

import { editorMode } from "./index"

describe("editorMode", () => {
  it("registers Search DSL without presenting it as SQL", () => {
    expect(editorMode("search-dsl")).not.toBeNull()
  })
})
