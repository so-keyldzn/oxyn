/**
 * The characters that delimit a name, as the dialect reads them: `"` for
 * PostgreSQL and SQLite, a backtick for MySQL, and both for ClickHouse,
 * which also escapes with a backslash inside them (RESEARCH-NOTES,
 * ClickHouse lexical rules).
 */
export type IdentifierQuote = '"' | "`" | '`"'

/** The same protocol choice governs editor navigation and answer diagrams. */
export function identifierQuoteForDriver(driver: string): IdentifierQuote {
  if (driver === "clickhouse") return '`"'
  return driver === "mysql" ? "`" : '"'
}

/** One part of a dotted name: `public`, or `"Order Lines"` without quotes. */
export interface NamePart {
  text: string
  /** Written between identifier quotes: compared exactly, case included. */
  quoted: boolean
}

/**
 * How a name typed without quotes is compared with a stored one. The rule is
 * the dialect's, and the host says which (the snapshot's `folding`):
 *
 * - `lower`: PostgreSQL folds the ASCII letters to lowercase and compares
 *   exactly, so `users` is never `"Users"`;
 * - `insensitive`: the others, whose rule depends on a server setting or
 *   ignores case.
 */
export type Folding = "lower" | "insensitive"

/**
 * Whether `part`, as typed, names `name`: a quoted part exactly, an unquoted
 * one by `folding`. Without it, case is ignored — the rule of a name matched
 * against the tree, which knows no dialect.
 */
export function sameName(
  part: NamePart,
  name: string | null,
  folding: Folding = "insensitive"
) {
  if (name === null) return false
  if (part.quoted) return part.text === name
  return folding === "lower"
    ? part.text.replace(/[A-Z]/g, (letter) => letter.toLowerCase()) === name
    : part.text.toLowerCase() === name.toLowerCase()
}

const WORD = /[\p{L}\p{N}_$]/u

/** A chain of parts separated by dots, starting at `index`. */
export function readIdentifierChain(
  line: string,
  index: number,
  identifierQuote: IdentifierQuote
) {
  const parts: Array<NamePart> = []
  let position = index
  for (;;) {
    const part = readPart(line, position, identifierQuote)
    if (part === null) break
    parts.push(part.part)
    position = part.end
    if (line[position] !== ".") break
    // A dot followed by nothing that names is not part of the name.
    if (readPart(line, position + 1, identifierQuote) === null) break
    position += 1
  }
  return parts.length === 0 ? null : { parts, end: position }
}

function readPart(
  line: string,
  index: number,
  identifierQuote: IdentifierQuote
) {
  const delimiter = line[index]
  if (delimiter !== undefined && identifierQuote.includes(delimiter)) {
    // Only ClickHouse escapes with a backslash inside a name.
    const backslash = identifierQuote.length > 1
    let text = ""
    let position = index + 1
    while (position < line.length) {
      const next = line[position + 1]
      if (backslash && line[position] === "\\" && next !== undefined) {
        text += next
        position += 2
        continue
      }
      if (line[position] === delimiter) {
        // A doubled delimiter is a quote inside the name.
        if (next === delimiter) {
          text += delimiter
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
