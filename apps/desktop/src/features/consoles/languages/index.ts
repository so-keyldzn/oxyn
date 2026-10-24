import type { Extension } from "@codemirror/state"

import type { QueryLanguageName } from "@/lib/query-language"

import { searchDslEditorMode } from "./search-dsl/search-dsl-mode"

/**
 * The editor mode of each language that is not SQL — highlighting, and
 * whatever else its CodeMirror extension brings.
 *
 * A language's mode lives in `./<product>/` and joins this table by one line.
 * A language absent from it is edited as plain text: what the console sends
 * does not depend on its mode, only how it reads.
 */
const EDITOR_MODES: Partial<
  Record<Exclude<QueryLanguageName, "sql">, () => Extension>
> = {
  "search-dsl": searchDslEditorMode,
}

export function editorMode(
  language: Exclude<QueryLanguageName, "sql">
): Extension | null {
  return EDITOR_MODES[language]?.() ?? null
}
