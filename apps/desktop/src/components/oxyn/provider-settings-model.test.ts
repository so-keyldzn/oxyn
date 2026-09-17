import { describe, expect, it } from "vitest"

import {
  endpointCarriesCredentials,
  parseArguments,
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

  it("splits a command line without a shell", () => {
    expect(
      parseArguments(`--model "claude sonnet" --flag='a b' $HOME`)
    ).toEqual(["--model", "claude sonnet", "--flag=a b", "$HOME"])
    expect(parseArguments(`''`)).toEqual([""])
  })
})
