import type { Extension } from "@codemirror/state"

import { mongoMode } from "./mongo"
import { redisEditorMode } from "./redis/redis-mode"
import type { CompletionCatalog } from "@/lib/ipc/completion"
import type { QueryLanguageName } from "@/lib/query-language"

import { searchDslEditorMode } from "./search-dsl/search-dsl-mode"

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
  redis: redisEditorMode,
  "search-dsl": searchDslEditorMode,
}

export function editorMode(
  language: Exclude<QueryLanguageName, "sql">,
  context: EditorModeContext
): Extension | null {
  return EDITOR_MODES[language]?.(context) ?? null
}

/** What a console of a language is called and starts with. */
export interface ConsoleFile {
  /** The end of its title: `console_1.<extension>`, `History copy.<extension>`. */
  extension: string
  /** The text a new console opens with — never run, and valid in the language. */
  starter: string
}

/**
 * A language's console file, joining this table by one line like its mode.
 * A language absent from it gets plain text and an empty console: a SQL
 * starter there would be a first command the server refuses.
 */
const CONSOLE_FILES: Partial<Record<QueryLanguageName, ConsoleFile>> = {
  sql: { extension: "sql", starter: "SELECT 1;" },
  // Redis commands have no comment syntax: any starter would be a command.
  redis: { extension: "redis", starter: "" },
  // A console holds Extended JSON commands; `ping` reads nothing.
  mongo: { extension: "json", starter: '{"ping": 1}' },
  // A request line, then its body: the shape of an HTTP request file. The
  // cluster's banner reads nothing an index holds.
  "search-dsl": { extension: "http", starter: "GET /" },
}

const PLAIN_TEXT: ConsoleFile = { extension: "txt", starter: "" }

/** `null`: a session that declares no language, or several besides SQL. */
export function consoleFile(language: QueryLanguageName | null): ConsoleFile {
  return (language === null ? undefined : CONSOLE_FILES[language]) ?? PLAIN_TEXT
}
