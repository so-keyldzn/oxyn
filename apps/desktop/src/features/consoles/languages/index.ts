import type { Extension } from "@codemirror/state"

import type { QueryLanguageName } from "@/lib/query-language"

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
  // Search DSL's endpoint line and JSON body need a dedicated mixed parser.
  // Until it exists, register the language explicitly as plain text rather
  // than applying SQL highlighting to a request the server would not parse.
  "search-dsl": () => [],
}

export function editorMode(
  language: Exclude<QueryLanguageName, "sql">
): Extension | null {
  return EDITOR_MODES[language]?.() ?? null
}
