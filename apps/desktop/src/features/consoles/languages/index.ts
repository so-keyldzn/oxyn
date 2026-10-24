import type { Extension } from "@codemirror/state"

import { mongoMode } from "./mongo"
import type { CompletionCatalog } from "@/lib/ipc/completion"
import type { QueryLanguageName } from "@/lib/query-language"

/** What the editor lends a mode. */
export interface EditorModeContext {
  /**
   * The latest snapshot of the catalog cache (ADR-0057), read on each
   * completion request; null while it is not read.
   */
  catalog: () => CompletionCatalog | null
}

/**
 * The editor mode of each language that is not SQL — highlighting, and
 * whatever else its CodeMirror extension brings.
 *
 * A language's mode lives in `./<product>/` and joins this table by one line.
 * A language absent from it is edited as plain text: what the console sends
 * does not depend on its mode, only how it reads.
 */
const EDITOR_MODES: Partial<
  Record<
    Exclude<QueryLanguageName, "sql">,
    (context: EditorModeContext) => Extension
  >
> = {
  mongo: mongoMode,
}

export function editorMode(
  language: Exclude<QueryLanguageName, "sql">,
  context: EditorModeContext
): Extension | null {
  return EDITOR_MODES[language]?.(context) ?? null
}
