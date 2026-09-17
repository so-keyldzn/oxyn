import { describe, expect, it } from "vitest"

import { refusalMessage, toAgentSettingChange } from "./agent-settings"

describe("a settings intent, as the command takes it", () => {
  it("sends a mode by its id alone", () => {
    expect(toAgentSettingChange({ kind: "mode", mode: "plan" })).toStrictEqual({
      mode: "plan",
    })
  })

  it("sends a select as the option and the chosen choice's id", () => {
    expect(
      toAgentSettingChange({ kind: "select", option: "model", choice: "opus" })
    ).toStrictEqual({ option: "model", value: "opus" })
  })

  it("sends a switch as the option and a boolean, never a string", () => {
    const change = toAgentSettingChange({
      kind: "boolean",
      option: "search",
      on: false,
    })
    expect(change).toStrictEqual({ option: "search", value: false })
    // `false` must not be turned into "false" on the way: the agent would read
    // a select value instead of a switch.
    expect(typeof (change as { value: unknown }).value).toBe("boolean")
  })
})

describe("a refusal, as the user reads it", () => {
  it("says what happened for every reason", () => {
    for (const reason of [
      "questionInProgress",
      "unknownMode",
      "unknownOption",
      "unknownValue",
      "byAgent",
      "unknown",
    ] as const) {
      const message = refusalMessage({ type: "refused", reason, code: null })
      expect(message.length).toBeGreaterThan(10)
      expect(message).not.toMatch(/^\d+$/)
    }
  })

  it("keeps the agent's code beside a sentence, never alone", () => {
    const message = refusalMessage({
      type: "refused",
      reason: "byAgent",
      code: -32602,
    })
    expect(message).toBe("the agent refused the change (error code -32602).")
  })

  it("does not pretend to know a reason this build cannot read", () => {
    expect(
      refusalMessage({ type: "refused", reason: "unknown", code: 7 })
    ).not.toContain("7")
  })
})
