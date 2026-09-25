import { describe, expect, it } from "vitest"

import { windowConsoles } from "./window-consoles"
import type { WorkspaceDocuments } from "./window-consoles"

function workspaces(
  ...held: Array<[string, WorkspaceDocuments]>
): ReadonlyMap<string, WorkspaceDocuments> {
  return new Map(held)
}

describe("windowConsoles", () => {
  it("lists every workspace's consoles in opening order, each once", () => {
    const report = windowConsoles(
      workspaces(
        ["billing", { documents: ["a", "b"], active: "b", visible: false }],
        ["analytics", { documents: ["c", "a"], active: null, visible: true }]
      )
    )
    expect(report.documents).toEqual(["a", "b", "c"])
  })

  it("takes the console in front from the workspace shown, and only from it", () => {
    const hidden = { documents: ["a"], active: "a", visible: false }
    expect(windowConsoles(workspaces(["billing", hidden])).active).toBeNull()
    expect(
      windowConsoles(
        workspaces(
          ["billing", hidden],
          ["analytics", { documents: ["c"], active: "c", visible: true }]
        )
      ).active
    ).toBe("c")
  })

  it("reports an empty window as such, so its consoles leave the file", () => {
    expect(windowConsoles(workspaces())).toEqual({
      documents: [],
      active: null,
    })
  })
})
