import { describe, expect, it } from "vitest"

import { relationMark } from "./relation-mark"

const plain = { virtualTable: null, shadowOf: null }

describe("relationMark", () => {
  it("leaves an ordinary table unmarked", () => {
    expect(relationMark(plain)).toBeNull()
  })

  it("names the module of a virtual table", () => {
    expect(
      relationMark({
        ...plain,
        virtualTable: { module: "fts5", available: true, shadows: [] },
      })
    ).toMatchObject({ label: "virtual · fts5", warning: false })
  })

  it("warns when the module is not loaded, whatever its name", () => {
    for (const module of ["vec0", "spellfix1"]) {
      const mark = relationMark({
        ...plain,
        virtualTable: { module, available: false, shadows: [] },
      })
      expect(mark).toMatchObject({
        label: `${module} not loaded`,
        warning: true,
      })
      expect(mark?.description).toContain("does not load")
    }
  })

  it("does not presume availability the engine could not report", () => {
    const mark = relationMark({
      ...plain,
      virtualTable: { module: "vec0", available: null, shadows: [] },
    })
    expect(mark).toMatchObject({ label: "virtual · vec0", warning: false })
    expect(mark?.description).toContain("unknown")
  })

  it("says whose data a shadow table stores", () => {
    expect(relationMark({ ...plain, shadowOf: "chunks_vec" })).toEqual({
      label: "shadow",
      description: "Stores the data of the virtual table chunks_vec.",
      warning: false,
    })
  })
})
