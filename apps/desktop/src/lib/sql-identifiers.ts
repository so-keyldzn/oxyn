export type IdentifierQuote = '"' | "`"

/** The same protocol choice governs editor navigation and answer diagrams. */
export function identifierQuoteForDriver(driver: string): IdentifierQuote {
  return driver === "mysql" ? "`" : '"'
}

/** One part of a dotted name: `public`, or `"Order Lines"` without quotes. */
export interface NamePart {
  text: string
  /** Written between identifier quotes: compared exactly, case included. */
  quoted: boolean
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
  if (line[index] === identifierQuote) {
    let text = ""
    let position = index + 1
    while (position < line.length) {
      if (line[position] === identifierQuote) {
        // A doubled delimiter is a quote inside the name.
        if (line[position + 1] === identifierQuote) {
          text += identifierQuote
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
