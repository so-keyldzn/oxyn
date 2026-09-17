import { describe, expect, it } from "vitest"

import { previewUnavailable } from "./capabilities"
import { previewStatus } from "./use-preview"
import { initialObjectTab } from "@/components/oxyn/object-view-frame"

describe("previewStatus", () => {
  const populated = {
    status: "populated" as const,
    result: "r1",
    columns: [],
    rows: 0,
    complete: true,
    truncated: false,
    cancelled: false,
  }

  it("calls a complete read without rows empty", () => {
    expect(previewStatus(populated)).toBe("empty")
  })

  it("does not call a read still streaming empty", () => {
    expect(previewStatus({ ...populated, complete: false })).toBe("loaded")
    expect(previewStatus({ ...populated, rows: 3 })).toBe("loaded")
  })

  it("maps a running read and a refused one", () => {
    expect(
      previewStatus({ status: "running", rows: 0, serverCancel: true })
    ).toBe("loading")
    expect(
      previewStatus({
        status: "error",
        message: "syntax error",
        retryable: false,
      })
    ).toBe("failed")
    expect(previewStatus({ status: "initial" })).toBe("initial")
  })
})

describe("the tab an object opens on", () => {
  const open = (capabilities: Array<string>) =>
    ({ capabilities }) as unknown as Parameters<typeof previewUnavailable>[0]

  it("never selects a disabled Data tab", () => {
    expect(initialObjectTab(undefined, false)).toBe("structure")
    expect(initialObjectTab("data", false)).toBe("structure")
    expect(initialObjectTab(undefined, true)).toBe("data")
    expect(initialObjectTab("definition", true)).toBe("definition")
  })

  it("says why Data is unavailable", () => {
    expect(previewUnavailable(open(["SQL"]), true)).toBeNull()
    expect(previewUnavailable(open(["SQL"]), false)).toMatch(/no rows/)
    expect(previewUnavailable(open([]), true)).toMatch(/cannot read rows/)
  })
})
