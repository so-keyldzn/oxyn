import { describe, expect, it } from "vitest"

import { consoleAvailability } from "./behaviour"
import type { ConsoleState } from "./context"
import { NO_FOCUS } from "./context"
import { availability } from "./registry"

const idle: ConsoleState = {
  canRun: true,
  canExplain: true,
  running: false,
  cancelling: false,
  writing: false,
}

const NO_PLAN = {
  reason: "This connection does not describe a statement's plan",
}

describe("Explain", () => {
  it("is offered where the session explains and nothing runs", () => {
    expect(consoleAvailability("explain", idle)).toBe(true)
    expect(consoleAvailability("explain", { ...idle, running: true })).toEqual({
      reason: "A query is running",
    })
  })

  it("is greyed with its reason where the session does not explain", () => {
    const silent = { ...idle, canExplain: false }
    expect(consoleAvailability("explain", silent)).toEqual(NO_PLAN)
    // The lasting reason, not the passing one.
    expect(
      consoleAvailability("explain", { ...silent, running: true })
    ).toEqual(NO_PLAN)
    // Run is untouched: a command still runs.
    expect(consoleAvailability("run", silent)).toBe(true)
  })

  it("is greyed the same way in the menu bar, the palette and the keyboard", () => {
    const context = {
      ...NO_FOCUS,
      platform: "mac" as const,
      sources: {
        console: {
          state: { ...idle, canExplain: false },
          actions: {
            run: () => undefined,
            runAll: () => undefined,
            explain: () => undefined,
            cancel: () => undefined,
            save: () => undefined,
          },
        },
      },
    }
    expect(availability("console.explain", context)).toEqual(NO_PLAN)
    expect(availability("console.run", context)).toBe(true)
  })
})
