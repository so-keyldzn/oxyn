import { describe, expect, it } from "vitest"

import { missingModuleSubject, sameModule } from "./missing-module-notice"

describe("sameModule", () => {
  it("folds ASCII case, as SQLite does", () => {
    expect(sameModule("VEC0", "vec0")).toBe(true)
    expect(sameModule("Fts5", "fts5")).toBe(true)
    expect(sameModule("vec0", "vec1")).toBe(false)
  })

  it("folds nothing beyond ASCII", () => {
    // SQLite keeps these apart: a Unicode fold would merge them.
    expect(sameModule("É", "é")).toBe(false)
    expect(sameModule("K", "K")).toBe(false)
  })
})

describe("missingModuleSubject", () => {
  const vtab = {
    kind: "table",
    virtualTable: { module: "VEC0", available: false, shadows: ["t_info"] },
  }

  it("recognizes the virtual table whatever the case of its module", () => {
    expect(missingModuleSubject(vtab, "vec0")).toEqual({
      type: "virtualTable",
      shadows: ["t_info"],
    })
  })

  it("treats anything else as reading such a table", () => {
    expect(
      missingModuleSubject({ kind: "view", virtualTable: null }, "vec0")
    ).toEqual({ type: "dependent", noun: "view" })
    expect(missingModuleSubject(vtab, "fts5")).toEqual({
      type: "dependent",
      noun: "object",
    })
  })
})
