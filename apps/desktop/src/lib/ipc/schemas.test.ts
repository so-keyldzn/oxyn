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
})
