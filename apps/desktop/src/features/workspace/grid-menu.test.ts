import { describe, expect, it } from "vitest"

import { gridAiLevel } from "./grid-menu"

describe("gridAiLevel", () => {
  it("offers nothing without a declared destination", () => {
    expect(gridAiLevel("absent", "sampled")).toEqual({ kind: "none" })
  })

  it("offers nothing for a connection it does not know", () => {
    expect(gridAiLevel("enabled", null)).toEqual({ kind: "none" })
  })

  it("lets values through the sample approval only under Sampled", () => {
    expect(gridAiLevel("enabled", "sampled")).toEqual({ kind: "sampled" })
    expect(gridAiLevel("disabled", "metadata")).toEqual({
      kind: "withheld",
      label: "Metadata",
    })
    expect(gridAiLevel("enabled", "local")).toEqual({
      kind: "withheld",
      label: "Local",
    })
  })
})
