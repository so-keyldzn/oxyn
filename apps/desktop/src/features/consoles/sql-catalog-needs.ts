// When the editor tells the host what the statement under the cursor needs of
// the catalog cache (ADR-0057 § 2).
//
// After a pause in typing, never on a keystroke: each keystroke only restarts
// the clock, and the statement is read — the tree walked — once it stops.

import { ViewPlugin } from "@codemirror/view"
import type { Extension } from "@codemirror/state"
import type { EditorView, ViewUpdate } from "@codemirror/view"

import { wantedNames } from "./sql-completion"
import type { NamePart } from "@/lib/sql-identifiers"

/**
 * How long the editor must be quiet before the host is asked to read. A burst
 * of typing is one request; a longer pause than this is the user reading what
 * was suggested.
 */
export const PAUSE_MS = 300

/**
 * Calls `report` with the names of the statement under the cursor, once the
 * text and the cursor have not moved for [`PAUSE_MS`] — and once when the
 * editor opens, so that a console edited before the tree was expanded has what
 * it needs by the first keystroke.
 *
 * `report` is read through `current` on each pause, so the extension is built
 * once; it returns `undefined` for a view that asks nothing (a read-only one).
 */
export function catalogNeeds(
  current: () => ((names: Array<Array<NamePart>>) => void) | undefined
): Extension {
  return ViewPlugin.fromClass(
    class {
      private timer: ReturnType<typeof setTimeout> | null = null

      constructor(view: EditorView) {
        this.restart(view)
      }

      update(update: ViewUpdate) {
        if (update.docChanged || update.selectionSet) this.restart(update.view)
      }

      destroy() {
        this.stop()
      }

      private stop() {
        if (this.timer === null) return
        clearTimeout(this.timer)
        this.timer = null
      }

      private restart(view: EditorView) {
        this.stop()
        if (current() === undefined) return
        this.timer = setTimeout(() => {
          this.timer = null
          current()?.(wantedNames(view.state, view.state.selection.main.head))
        }, PAUSE_MS)
      }
    }
  )
}
