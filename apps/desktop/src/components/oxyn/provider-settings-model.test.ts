import { describe, expect, it } from "vitest"

import {
  counted,
  endpointCarriesCredentials,
  parseArguments,
  parseEnvironment,
  reachSummary,
} from "./provider-settings-model"

describe("provider settings", () => {
  it("spots credentials in the authority only", () => {
    expect(
      endpointCarriesCredentials("https://alice:hunter2@api.example.com")
    ).toBe(true)
    expect(
      endpointCarriesCredentials("https://api.example.com/v1?user=a@b")
    ).toBe(false)
    expect(endpointCarriesCredentials("http://localhost:11434/v1")).toBe(false)
  })

  it("dates a classification and never rounds unresolved", () => {
    expect(reachSummary("unresolved", 0)).toBe(
      "Unresolved · measurement time unknown"
    )
    expect(reachSummary("local", Date.UTC(2026, 8, 15, 12, 0))).toMatch(
      /^Local · measured /
    )
  })

  it("takes one argument per line, exactly as typed", () => {
    expect(
      parseArguments(`-y\n--model "claude sonnet"\r\n\n; rm -rf /\n$HOME`)
    ).toEqual(["-y", '--model "claude sonnet"', "; rm -rf /", "$HOME"])
  })

  it("reads NAME=value lines and refuses a line without a name", () => {
    expect(parseEnvironment("A_B=x=y\n\nEMPTY=\r\n")).toEqual({
      ok: true,
      env: [
        { name: "A_B", value: "x=y" },
        { name: "EMPTY", value: "" },
      ],
    })
    expect(parseEnvironment("OK=1\n1BAD=2")).toEqual({ ok: false, line: 2 })
    expect(parseEnvironment("no equals")).toEqual({ ok: false, line: 1 })
  })

  it("counts in the singular only for one", () => {
    expect(counted(1, "model", "models")).toBe("1 model")
    expect(counted(0, "model", "models")).toBe("0 models")
    expect(counted(3, "argument", "arguments")).toBe("3 arguments")
  })
})
