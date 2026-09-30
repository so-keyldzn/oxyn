// `Open object under cursor` (UX-SPEC, « Context menus », Éditeur SQL):
// the name under the cursor, resolved against the catalog already read — the
// tree the sidebar holds —, as a mention resolves what is typed after `@`.
// Nothing is read from the server for it, and nothing runs.

import type { CatalogAddress, CatalogNode } from "@/lib/ipc/types"

import { readIdentifierChain } from "@/lib/sql-identifiers"
import type { IdentifierQuote, NamePart } from "@/lib/sql-identifiers"

/**
 * The dotted name that covers `offset` in `text` — `orders`, `public.orders`,
 * `"Order Lines"` — or null when the offset sits on none.
 *
 * Only the offset's line is read: a SQL name never spans lines.
 */
export function nameAt(
  text: string,
  offset: number,
  identifierQuote: IdentifierQuote = '"'
): Array<NamePart> | null {
  const lineStart = text.lastIndexOf("\n", offset - 1) + 1
  const newline = text.indexOf("\n", offset)
  const line = text.slice(lineStart, newline < 0 ? text.length : newline)
  const at = offset - lineStart
  let index = 0
  while (index < line.length) {
    const quote = line[index]
    // A MySQL double-quoted string must not resolve to a loaded table.
    if (quote === "'" || (quote === '"' && identifierQuote === "`")) {
      index += 1
      while (index < line.length) {
        if (line[index] === "\\" && identifierQuote === "`") index += 2
        else if (line[index] === quote) {
          index += 1
          if (line[index] !== quote) break
          index += 1
        } else index += 1
      }
      continue
    }
    const start = index
    const parts = readIdentifierChain(line, index, identifierQuote)
    if (parts === null) {
      index += 1
      continue
    }
    index = parts.end
    // The cursor right after the last character still names it.
    if (at >= start && at <= parts.end) return parts.parts
  }
  return null
}

function same(part: NamePart, name: string | null) {
  if (name === null) return false
  return part.quoted
    ? part.text === name
    : part.text.toLowerCase() === name.toLowerCase()
}

/**
 * The relation of the loaded tree that `parts` names, or null when none does
 * or when several do: an ambiguous name opens nothing rather than a guess.
 *
 * Only what the tree has read is searched — a schema never expanded holds no
 * relation yet —, and system objects are left out as the backend flags them.
 * An unquoted name is compared without case, as the servers Oxyn speaks to
 * fold it; an exact match wins over one that differs by case only.
 */
export function loadedObject(
  nodes: ReadonlyArray<CatalogNode>,
  parts: ReadonlyArray<NamePart>
): CatalogAddress | null {
  const relation = parts[parts.length - 1]
  if (relation === undefined || parts.length > 3) return null
  const namespace = parts.length >= 2 ? parts[parts.length - 2] : undefined
  const catalog = parts.length === 3 ? parts[0] : undefined
  const found: Array<CatalogNode> = []
  const walk = (level: ReadonlyArray<CatalogNode>) => {
    for (const node of level) {
      if (node.system) continue
      if (node.address.relation !== null) {
        if (
          same(relation, node.name) &&
          (namespace === undefined ||
            same(namespace, node.address.namespace)) &&
          (catalog === undefined || same(catalog, node.address.catalog))
        )
          found.push(node)
      } else walk(node.children)
    }
  }
  walk(nodes)
  if (found.length === 1) return found[0]?.address ?? null
  const exact = found.filter((node) => node.name === relation.text)
  return exact.length === 1 ? (exact[0]?.address ?? null) : null
}
