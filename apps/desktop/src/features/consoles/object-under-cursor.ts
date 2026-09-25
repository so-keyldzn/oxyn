// `Open object under cursor` (UX-SPEC, « Menus contextuels », Éditeur SQL):
// the name under the cursor, resolved against the catalog already read — the
// tree the sidebar holds —, as a mention resolves what is typed after `@`.
// Nothing is read from the server for it, and nothing runs.

import type { CatalogAddress, CatalogNode } from "@/lib/ipc/types"

/** One part of a dotted name: `public`, or `"Order Lines"` without quotes. */
export interface NamePart {
  text: string
  /** Written between double quotes: compared exactly, case included. */
  quoted: boolean
}

const WORD = /[\p{L}\p{N}_$]/u

/**
 * The dotted name that covers `offset` in `text` — `orders`, `public.orders`,
 * `"Order Lines"` — or null when the offset sits on none.
 *
 * Only the offset's line is read: a SQL name never spans lines.
 */
export function nameAt(text: string, offset: number): Array<NamePart> | null {
  const lineStart = text.lastIndexOf("\n", offset - 1) + 1
  const newline = text.indexOf("\n", offset)
  const line = text.slice(lineStart, newline < 0 ? text.length : newline)
  const at = offset - lineStart
  let index = 0
  while (index < line.length) {
    const start = index
    const parts = readChain(line, index)
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

/** A chain of parts separated by dots, starting at `index`. */
function readChain(line: string, index: number) {
  const parts: Array<NamePart> = []
  let position = index
  for (;;) {
    const part = readPart(line, position)
    if (part === null) break
    parts.push(part.part)
    position = part.end
    if (line[position] !== ".") break
    // A dot followed by nothing that names is not part of the name.
    if (readPart(line, position + 1) === null) break
    position += 1
  }
  return parts.length === 0 ? null : { parts, end: position }
}

function readPart(line: string, index: number) {
  if (line[index] === '"') {
    let text = ""
    let position = index + 1
    while (position < line.length) {
      if (line[position] === '"') {
        // `""` is a quote inside the name.
        if (line[position + 1] === '"') {
          text += '"'
          position += 2
          continue
        }
        return text === ""
          ? null
          : { part: { text, quoted: true }, end: position + 1 }
      }
      text += line[position]
      position += 1
    }
    return null
  }
  let position = index
  while (position < line.length && WORD.test(line[position] ?? ""))
    position += 1
  // A number is not a name.
  if (position === index || /^\p{N}/u.test(line[index] ?? "")) return null
  return {
    part: { text: line.slice(index, position), quoted: false },
    end: position,
  }
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
