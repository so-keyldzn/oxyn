import { describe, expect, it } from "vitest"

import { diagnosticText } from "@/lib/diagnostic-info"
import { BuildIdentity } from "@/lib/ipc/about"

const SHA = "8359edb9ecae7f7bc5fa0fd0006b48634a2694c6"

describe("diagnosticText", () => {
  it("says the identity the build recorded, and nothing else", () => {
    // What a future backend might add to the response: a host, a path, a
    // variable. The schema drops it, and the text names its fields anyway.
    const response = BuildIdentity.parse({
      version: "0.0.1",
      revision: SHA,
      modified: false,
      os: "macos",
      arch: "aarch64",
      profile: "release",
      host: "db.internal:5432",
      home: "/Users/someone",
      env: { OPENAI_API_KEY: "sk-do-not-leak" },
    })

    expect(diagnosticText(response)).toBe(
      [
        "Oxyn",
        "Version: 0.0.1",
        `Source revision: ${SHA}`,
        "Modified sources: No",
        "Platform: macos aarch64",
        "Build: Release",
      ].join("\n")
    )
  })

  it("says Unknown for what the build did not record", () => {
    const text = diagnosticText({
      version: "0.0.1",
      revision: null,
      modified: null,
      os: "linux",
      arch: "x86_64",
      profile: "development",
    })
    expect(text).toContain("Source revision: Unknown")
    expect(text).toContain("Modified sources: Unknown")
    expect(text).toContain("Build: Development")
  })

  it("invents nothing when the identity could not be read", () => {
    expect(diagnosticText(null)).toBe(
      [
        "Oxyn",
        "Version: Unknown",
        "Source revision: Unknown",
        "Modified sources: Unknown",
        "Platform: Unknown",
        "Build: Unknown",
      ].join("\n")
    )
  })
})
