// `Open object under cursor` in a MongoDB console (UX-SPEC, « Context
// menus »): a command names its collection as a JSON string — `"find":
// "orders"`, `"from": "items"` in a `$lookup` —, never as a SQL identifier, so
// the name is the string value under the cursor. The database is the
// command's `$db` when it names one; otherwise the collection is looked for
// in every database loaded, and only a single match opens.

import { readJson } from "./json-position"
import type { NamePart } from "@/lib/sql-identifiers"

/**
 * The collection name a string value under `offset` gives — preceded by the
 * command's `$db` when it has one — or null off a closed string value.
 *
 * Both parts are quoted: a MongoDB name is compared exactly, case included.
 */
export function mongoNameAt(
  text: string,
  offset: number
): Array<NamePart> | null {
  const { slot, rootStrings } = readJson(text, offset)
  if (slot?.role !== "value" || !slot.quoted || !slot.closed) return null
  const literal = text.slice(slot.from - 1, slot.to + 1)
  let name: unknown
  try {
    name = JSON.parse(literal)
  } catch {
    return null
  }
  if (typeof name !== "string" || name === "") return null
  // A `$db` value is a database, not a collection.
  if (slot.route.length === 0 && slot.key === "$db") return null
  const database = rootStrings.get("$db")
  const collection = { text: name, quoted: true }
  return database
    ? [{ text: database, quoted: true }, collection]
    : [collection]
}
