import { describe, expect, it } from "vitest"

import {
  closeMessage,
  consoleTitle,
  cycleConsole,
  needsCloseDecision,
  saveNotice,
  tabState,
  tabTitle,
  titleTooLong,
} from "./console-model"

describe("console model", () => {
  it("names consoles like the GPUI workspace", () => {
    expect(consoleTitle(3)).toBe("console_3.sql")
    expect(tabTitle({ title: " ", fromAgent: true })).toBe(
      "AI · Untitled query"
    )
  })

  it("shows the most important activity first", () => {
    expect(tabState({ activity: "running", unsaved: true })).toBe("running")
    expect(tabState({ activity: "idle", unsaved: true })).toBe("unsaved")
    expect(tabState({ activity: "idle", unsaved: false })).toBeNull()
  })

  it("asks before losing text or stopping a statement", () => {
    const quiet = {
      title: "q.sql",
      conflict: false,
      hasSavedCopy: true,
      unsaved: false,
      running: false,
    }
    expect(needsCloseDecision(quiet)).toBe(false)
    expect(needsCloseDecision({ ...quiet, running: true })).toBe(true)
    const message = closeMessage({ ...quiet, unsaved: true, running: true })
    expect(message).toContain("Closing discards this text.")
    expect(message).toContain("does not undo committed database changes")
  })

  it("says where the saved copy stands", () => {
    expect(saveNotice({ hasSavedCopy: false, unsaved: true })).toBe(
      "No saved copy yet."
    )
    expect(saveNotice({ hasSavedCopy: true, unsaved: false })).toBe(
      "Matches the saved version."
    )
  })

  it("cycles consoles and ignores a lone one", () => {
    expect(cycleConsole(["a"], "a", 1)).toBeNull()
    expect(cycleConsole(["a", "b", "c"], "c", 1)).toBe("a")
    expect(cycleConsole(["a", "b", "c"], "a", -1)).toBe("c")
  })

  it("bounds a title in bytes, not characters", () => {
    expect(titleTooLong("é".repeat(129))).toBe(true)
    expect(titleTooLong("é".repeat(128))).toBe(false)
  })
})
