import { MONGO_FENCES } from "./mongo/fences"
import type { QueryLanguageName } from "@/lib/query-language"

/**
 * The fence labels, lower-cased, of a code block a console of each language
 * may open. SQL takes an unlabelled fence too: its agent answers with
 * queries. A language absent from this table opens nothing — a block it
 * cannot name would land in a console that reads it as something else.
 *
 * A language's labels live in `./<product>/` and join this table by one line.
 */
const CONSOLE_FENCES: Partial<
  Record<QueryLanguageName, ReadonlyArray<string>>
> = {
  sql: ["", "sql"],
  mongo: MONGO_FENCES,
}

/** Does a fence labelled `fence` hold text a `language` console runs? */
export function fenceOpensIn(
  fence: string,
  language: QueryLanguageName | null
): boolean {
  if (language === null) return false
  return CONSOLE_FENCES[language]?.includes(fence.toLowerCase()) ?? false
}
