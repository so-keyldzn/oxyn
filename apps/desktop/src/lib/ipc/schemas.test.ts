import { describe, expect, it } from "vitest"

import * as ai from "./ai"
import * as client from "./client"
import * as consoles from "./consoles"
import * as events from "./events"
import * as library from "./library"
import * as metadata from "./metadata"
import * as recovery from "./recovery"
import * as results from "./results"
import * as settings from "./settings"
import * as types from "./types"

// Schemas are values, not types: a cycle between two of these modules is no
// longer erased at compile time the way `import type` erased it. It shows up
// as a schema that is `undefined` when its module is evaluated — at load, far
// from the call that fails. `ConsoleSession` was moved into `types.ts` and
// `listConnections` into `settings.ts` for exactly this reason (ADR-0031).

const modules = {
  ai,
  client,
  consoles,
  events,
  library,
  metadata,
  recovery,
  results,
  settings,
  types,
}

describe("the boundary's modules", () => {
  it("evaluate every exported schema, so no import cycle is left open", () => {
    const undefinedExports: Array<string> = []
    for (const [name, module] of Object.entries(modules)) {
      for (const [exported, value] of Object.entries(module)) {
        if (value === undefined) undefinedExports.push(`${name}.${exported}`)
      }
    }
    expect(undefinedExports).toEqual([])
  })

  it("parse with the schemas they export, rather than holding shapes", () => {
    // A cycle can also leave a schema half-built: present, but unable to
    // parse. Exercising one schema per module catches that.
    expect(types.Environment.safeParse("production").success).toBe(true)
    expect(
      consoles.SessionPlace.safeParse({
        catalog: null,
        namespace: null,
      }).success
    ).toBe(true)
    expect(settings.ThemeChoice.safeParse("dark").success).toBe(true)
    expect(
      results.ExportFormatChoice.safeParse({
        format: "csv",
        label: "CSV",
        extension: "csv",
        supported: true,
      }).success
    ).toBe(true)
  })

  it("mirror the copy commands' names exactly", () => {
    // The Rust enums are `camelCase`: `inList`, `insertTemplate`. A
    // snake_case spelling on either side fails every call, not a rare one.
    expect(results.CopyRowsFormat.options).toEqual([
      "tsv",
      "csv",
      "json",
      "markdown",
      "insert",
      "inList",
    ])
    expect(metadata.ObjectSqlForm.options).toEqual([
      "quotedName",
      "selectAll",
      "insertTemplate",
    ])
    expect(
      results.CopiedRows.safeParse({ text: "1\t2", rows: 1 }).success
    ).toBe(true)
    expect(results.CopiedRows.safeParse({ text: "", rows: -1 }).success).toBe(
      false
    )
    expect(
      results.CopyRowsRequest.safeParse({
        result: "r",
        offset: 0,
        count: 2,
        columns: [1, 0],
        format: "inList",
        header: false,
        connection: null,
        address: null,
      }).success
    ).toBe(true)
  })
})
