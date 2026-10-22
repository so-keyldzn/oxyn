// What the completion says when it is not whole (ADR-0057 § 6): a snapshot cut
// by a bound is never presented as complete. The note sits in a panel under the
// text, where CodeMirror reserves its own room, so it hides none of the SQL.

import { StateEffect, StateField } from "@codemirror/state"
import type { Extension } from "@codemirror/state"
import { showPanel } from "@codemirror/view"
import type { EditorView } from "@codemirror/view"

import type { CompletionCatalog } from "@/lib/ipc/completion"

function counted(count: number, one: string, many: string) {
  return `${count.toLocaleString("en")} ${count === 1 ? one : many}`
}

/** The note a partial snapshot carries, or null when nothing was left out. */
export function partialNote(catalog: CompletionCatalog | null): string | null {
  const cut = catalog?.truncated
  if (!cut) return null
  const left: Array<string> = []
  if (cut.namespacesLeftOut > 0)
    left.push(
      counted(
        cut.namespacesLeftOut,
        "schema or database",
        "schemas or databases"
      )
    )
  if (cut.relationsLeftOut > 0)
    left.push(counted(cut.relationsLeftOut, "table or view", "tables or views"))
  if (cut.fieldsLeftOut > 0)
    left.push(counted(cut.fieldsLeftOut, "column", "columns"))
  return `Suggestions are partial. Left out: ${left.join(", ")}.`
}

const setNote = StateEffect.define<string | null>()

const note = StateField.define<string | null>({
  create: () => null,
  update(value, transaction) {
    for (const effect of transaction.effects)
      if (effect.is(setNote)) return effect.value
    return value
  },
  provide: (field) =>
    showPanel.from(field, (text) =>
      text === null
        ? null
        : () => {
            const dom = document.createElement("div")
            dom.className = "cm-completion-note"
            // Counts only, composed here: no name of the catalog reaches it.
            dom.textContent = text
            return { dom }
          }
    ),
})

/** The panel, empty until [`showCompletionNote`] gives it a text. */
export const completionNote: Extension = note

export function showCompletionNote(view: EditorView, text: string | null) {
  if (view.state.field(note) !== text)
    view.dispatch({ effects: setNote.of(text) })
}
