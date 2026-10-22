import { EditorState } from "@codemirror/state"
import { EditorView, getPanel } from "@codemirror/view"
import { describe, expect, it } from "vitest"

import {
  completionNote,
  partialNote,
  showCompletionNote,
} from "./sql-completion-note"
import type { CompletionCatalog } from "@/lib/ipc/completion"

function snapshot(
  truncated: CompletionCatalog["truncated"]
): CompletionCatalog {
  return { namespaces: [], resolution: { state: "unknown" }, truncated }
}

describe("the note of a partial snapshot", () => {
  it("says nothing of a whole one, or of none", () => {
    expect(partialNote(null)).toBeNull()
    expect(partialNote(snapshot(null))).toBeNull()
  })

  it("counts only what was left out, in the singular too", () => {
    expect(
      partialNote(
        snapshot({
          namespacesLeftOut: 0,
          relationsLeftOut: 1,
          fieldsLeftOut: 2,
        })
      )
    ).toBe("Suggestions are partial. Left out: 1 table or view, 2 columns.")
    expect(
      partialNote(
        snapshot({
          namespacesLeftOut: 40,
          relationsLeftOut: 0,
          fieldsLeftOut: 0,
        })
      )
    ).toBe("Suggestions are partial. Left out: 40 schemas or databases.")
  })

  it("groups thousands", () => {
    expect(
      partialNote(
        snapshot({
          namespacesLeftOut: 0,
          relationsLeftOut: 47_000,
          fieldsLeftOut: 0,
        })
      )
    ).toContain("47,000 tables or views")
  })
})

describe("the panel", () => {
  function editor() {
    return new EditorView({
      state: EditorState.create({
        doc: "SELECT 1",
        extensions: [completionNote],
      }),
      parent: document.body,
    })
  }

  it("shows the note, replaces it, and goes away with it", () => {
    const view = editor()
    expect(getPanel(view, () => ({ dom: document.createElement("div") }))).toBe(
      null
    )
    showCompletionNote(view, "Suggestions are partial. Left out: 1 column.")
    expect(view.dom.querySelector(".cm-completion-note")?.textContent).toBe(
      "Suggestions are partial. Left out: 1 column."
    )
    showCompletionNote(view, "Suggestions are partial. Left out: 2 columns.")
    expect(view.dom.querySelectorAll(".cm-completion-note")).toHaveLength(1)
    expect(
      view.dom.querySelector(".cm-completion-note")?.textContent
    ).toContain("2 columns")
    showCompletionNote(view, null)
    expect(view.dom.querySelector(".cm-completion-note")).toBeNull()
    view.destroy()
  })
})
