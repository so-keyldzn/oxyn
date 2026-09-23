// A model's answer, parsed into a tree the renderer draws with React text.
//
// Why not a Markdown library: every one of them ends in HTML, then relies on a
// sanitizer to take back what the model should never have been able to write
// (docs/SECURITY.md, « Surface d'entrée »). Here there is no HTML at any stage:
// raw tags stay text, an image is its alternative text, a link is its label
// and its address shown as text. Nothing the model writes can load, navigate
// or run in the webview, and that is a property of the types below rather than
// of a configuration someone might loosen.
//
// The subset is what an SQL assistant writes: paragraphs, headings, lists,
// quotes, rules, pipe tables, fenced code, and inline code, emphasis and links.
// Unterminated constructs — an answer still streaming — stay readable text.

export type Inline =
  | { type: "text"; text: string }
  | { type: "code"; text: string }
  | { type: "strong"; children: Array<Inline> }
  | { type: "emphasis"; children: Array<Inline> }
  | { type: "deleted"; children: Array<Inline> }
  | { type: "link"; children: Array<Inline>; href: string }
  | { type: "image"; alt: string }

export type Block =
  | { type: "paragraph"; content: Array<Inline> }
  | { type: "heading"; level: number; content: Array<Inline> }
  | {
      type: "code"
      language: string
      text: string
      /** `false` while the closing fence has not arrived. */
      closed: boolean
    }
  | {
      type: "list"
      ordered: boolean
      start: number
      items: Array<Array<Inline>>
    }
  | { type: "quote"; blocks: Array<Block> }
  | { type: "rule" }
  | {
      type: "table"
      header: Array<Array<Inline>>
      rows: Array<Array<Array<Inline>>>
    }

const FENCE = /^ {0,3}(`{3,}|~{3,})\s*([^\s`]*)[^`]*$/
const HEADING = /^ {0,3}(#{1,6})\s+(.*?)\s*#*\s*$/
const RULE = /^ {0,3}([-*_])(\s*\1){2,}\s*$/
const BULLET = /^ {0,3}[-*+]\s+(.*)$/
const ORDERED = /^ {0,3}(\d{1,9})[.)]\s+(.*)$/
const QUOTE = /^ {0,3}>\s?(.*)$/
const TABLE_SEPARATOR = /^\s*\|?\s*:?-{3,}:?\s*(\|\s*:?-{3,}:?\s*)*\|?\s*$/

/** Is this block SQL a user may open in a console? */
export function isSqlBlock(
  block: Block
): block is Extract<Block, { type: "code" }> {
  if (block.type !== "code" || !block.closed || block.text.trim() === "")
    return false
  // An unlabelled fence counts: this agent answers with queries.
  const language = block.language.toLowerCase()
  return language === "" || language === "sql"
}

export type CodeBlock = Extract<Block, { type: "code" }>

/**
 * A table an `erd` block names, as the model wrote it. It is a request, not a
 * fact: the feature resolves it against the catalog, and a name the catalog
 * does not hold is shown as not found — never drawn from its spelling.
 */
export interface ErdName {
  /** `null` when the model did not qualify the name. */
  namespace: string | null
  relation: string
  /** The line as written, for the « not found » list. */
  written: string
}

/** What a closed `erd` block asks the feature to draw. */
export interface ErdRequest {
  names: Array<ErdName>
  /** Names past `ERD_MAX_NAMES`, not looked up. */
  ignoredNames: number
  /** The block's text, for « Show source ». */
  source: string
}

/** Names one `erd` block may ask for; the rest is announced, not drawn. */
export const ERD_MAX_NAMES = 20

/** A line longer than this is not a table name, whatever it says. */
const ERD_MAX_LINE = 256

/** Is this a closed `mermaid` block, one the answer may draw? */
export function isMermaidBlock(block: Block): block is CodeBlock {
  return (
    block.type === "code" &&
    block.closed &&
    block.language.toLowerCase() === "mermaid" &&
    block.text.trim() !== ""
  )
}

/** Is this a closed `erd` block, one the answer may draw as a diagram? */
export function isErdBlock(block: Block): block is CodeBlock {
  return (
    block.type === "code" &&
    block.closed &&
    block.language.toLowerCase() === "erd" &&
    block.text.trim() !== ""
  )
}

/**
 * Splits `schema.table` on the dots outside double quotes, as SQL spells a
 * qualified name: `"a.b".c` is two segments, and `""` inside quotes is one
 * quote. `null` for a name that is not one — an empty segment, an open quote.
 */
function erdSegments(name: string): Array<string> | null {
  const segments: Array<string> = []
  let current = ""
  let quoted = false
  let wasQuoted = false
  for (let index = 0; index < name.length; index += 1) {
    const char = name[index]
    if (quoted) {
      if (char === '"' && name[index + 1] === '"') {
        current += '"'
        index += 1
      } else if (char === '"') {
        quoted = false
      } else {
        current += char
      }
    } else if (char === '"') {
      quoted = true
      wasQuoted = true
    } else if (char === ".") {
      if (current === "") return null
      segments.push(current)
      current = ""
      wasQuoted = false
    } else {
      current += char
    }
  }
  if (quoted || (current === "" && !wasQuoted)) return null
  segments.push(current)
  return segments
}

/**
 * The tables an `erd` block names, one per line, possibly `schema.table`.
 *
 * Blank lines and comments (`--`, `#`, `//`) are skipped, as are list markers
 * and a trailing `;` or `,` a model tends to add. Beyond `ERD_MAX_NAMES`, the
 * names are counted in `dropped` so the diagram can say it is partial.
 */
export function parseErdNames(text: string): {
  names: Array<ErdName>
  dropped: number
} {
  const names: Array<ErdName> = []
  const seen = new Set<string>()
  let dropped = 0
  for (const raw of text.replace(/\r\n?/g, "\n").split("\n")) {
    const line = raw
      .trim()
      .replace(/^[-*+]\s+/, "")
      .replace(/[;,]\s*$/, "")
      .trim()
    if (
      line === "" ||
      line.startsWith("--") ||
      line.startsWith("#") ||
      line.startsWith("//") ||
      line.length > ERD_MAX_LINE
    )
      continue
    const segments = erdSegments(line)
    if (segments === null) continue
    // `catalog.schema.table`: the catalog is the connection's, not the model's.
    const relation = segments.at(-1) ?? ""
    const namespace = segments.length > 1 ? (segments.at(-2) ?? null) : null
    const key = JSON.stringify([namespace, relation])
    if (seen.has(key)) continue
    seen.add(key)
    if (names.length >= ERD_MAX_NAMES) {
      dropped += 1
      continue
    }
    names.push({ namespace, relation, written: line })
  }
  return { names, dropped }
}

function splitRow(line: string): Array<string> {
  let row = line.trim()
  if (row.startsWith("|")) row = row.slice(1)
  if (row.endsWith("|") && !row.endsWith("\\|")) row = row.slice(0, -1)
  const cells: Array<string> = []
  let cell = ""
  for (let index = 0; index < row.length; index += 1) {
    const char = row[index]
    if (char === "\\" && row[index + 1] === "|") {
      cell += "|"
      index += 1
    } else if (char === "|") {
      cells.push(cell.trim())
      cell = ""
    } else {
      cell += char
    }
  }
  cells.push(cell.trim())
  return cells
}

function startsBlock(line: string, next: string | undefined): boolean {
  return (
    FENCE.test(line) ||
    HEADING.test(line) ||
    RULE.test(line) ||
    BULLET.test(line) ||
    ORDERED.test(line) ||
    QUOTE.test(line) ||
    (line.includes("|") && next !== undefined && TABLE_SEPARATOR.test(next))
  )
}

export function parseMarkdown(source: string): Array<Block> {
  const lines = source.replace(/\r\n?/g, "\n").split("\n")
  const blocks: Array<Block> = []
  let index = 0

  while (index < lines.length) {
    const line = lines[index] ?? ""
    const next = lines[index + 1]

    if (line.trim() === "") {
      index += 1
      continue
    }

    const fence = FENCE.exec(line)
    if (fence) {
      const marker = fence[1] ?? "```"
      const body: Array<string> = []
      let closed = false
      index += 1
      while (index < lines.length) {
        const inner = lines[index] ?? ""
        const trimmed = inner.trim()
        if (
          trimmed.startsWith(marker.charAt(0).repeat(marker.length)) &&
          trimmed.replace(/[`~]/g, "") === ""
        ) {
          closed = true
          index += 1
          break
        }
        body.push(inner)
        index += 1
      }
      blocks.push({
        type: "code",
        language: fence[2] ?? "",
        text: body.join("\n"),
        closed,
      })
      continue
    }

    const heading = HEADING.exec(line)
    if (heading) {
      blocks.push({
        type: "heading",
        level: (heading[1] ?? "#").length,
        content: parseInline(heading[2] ?? ""),
      })
      index += 1
      continue
    }

    if (RULE.test(line)) {
      blocks.push({ type: "rule" })
      index += 1
      continue
    }

    if (QUOTE.test(line)) {
      const quoted: Array<string> = []
      while (index < lines.length) {
        const match = QUOTE.exec(lines[index] ?? "")
        if (!match) break
        quoted.push(match[1] ?? "")
        index += 1
      }
      blocks.push({ type: "quote", blocks: parseMarkdown(quoted.join("\n")) })
      continue
    }

    const bullet = BULLET.exec(line)
    const ordered = ORDERED.exec(line)
    if (bullet || ordered) {
      const isOrdered = ordered !== null && bullet === null
      const items: Array<Array<string>> = []
      while (index < lines.length) {
        const current = lines[index] ?? ""
        const item = isOrdered ? ORDERED.exec(current) : BULLET.exec(current)
        if (item) {
          items.push([(isOrdered ? item[2] : item[1]) ?? ""])
          index += 1
          continue
        }
        // A continuation line belongs to the item above.
        if (current.trim() !== "" && /^\s{2,}/.test(current) && items.length) {
          items.at(-1)?.push(current.trim())
          index += 1
          continue
        }
        break
      }
      blocks.push({
        type: "list",
        ordered: isOrdered,
        start:
          ordered !== null && bullet === null
            ? Number.parseInt(ordered[1] ?? "1", 10)
            : 1,
        items: items.map((parts) => parseInline(parts.join(" "))),
      })
      continue
    }

    if (
      line.includes("|") &&
      next !== undefined &&
      TABLE_SEPARATOR.test(next)
    ) {
      const header = splitRow(line).map(parseInline)
      const rows: Array<Array<Array<Inline>>> = []
      index += 2
      while (index < lines.length) {
        const row = lines[index] ?? ""
        if (row.trim() === "" || !row.includes("|")) break
        const cells = splitRow(row).map(parseInline)
        rows.push(header.map((_, column) => cells[column] ?? []))
        index += 1
      }
      blocks.push({ type: "table", header, rows })
      continue
    }

    const paragraph: Array<string> = [line.trim()]
    index += 1
    while (index < lines.length) {
      const current = lines[index] ?? ""
      if (current.trim() === "" || startsBlock(current, lines[index + 1])) break
      paragraph.push(current.trim())
      index += 1
    }
    blocks.push({
      type: "paragraph",
      content: parseInline(paragraph.join(" ")),
    })
  }

  return blocks
}

const PUNCTUATION = /[!-/:-@[-`{-~]/

function isWordChar(char: string | undefined) {
  return char !== undefined && /[\p{L}\p{N}]/u.test(char)
}

/** Finds what closes the bracket opened at `open`, honouring nesting. */
function closing(text: string, open: number, pair = "[]"): number {
  let depth = 0
  for (let index = open; index < text.length; index += 1) {
    const char = text[index]
    if (char === "\\") {
      index += 1
    } else if (char === pair[0]) {
      depth += 1
    } else if (char === pair[1]) {
      depth -= 1
      if (depth === 0) return index
    }
  }
  return -1
}

export function parseInline(text: string): Array<Inline> {
  const out: Array<Inline> = []
  let buffer = ""
  const flush = () => {
    if (buffer !== "") {
      const last = out.at(-1)
      if (last?.type === "text") last.text += buffer
      else out.push({ type: "text", text: buffer })
      buffer = ""
    }
  }

  let index = 0
  while (index < text.length) {
    const char = text[index] ?? ""

    if (char === "\\" && PUNCTUATION.test(text[index + 1] ?? "")) {
      buffer += text[index + 1]
      index += 2
      continue
    }

    if (char === "`") {
      let run = 1
      while (text[index + run] === "`") run += 1
      const marker = "`".repeat(run)
      const end = text.indexOf(marker, index + run)
      if (end !== -1) {
        flush()
        out.push({ type: "code", text: text.slice(index + run, end).trim() })
        index = end + run
        continue
      }
      buffer += marker
      index += run
      continue
    }

    if (char === "!" && text[index + 1] === "[") {
      const close = closing(text, index + 1)
      if (close !== -1 && text[close + 1] === "(") {
        const end = closing(text, close + 1, "()")
        if (end !== -1) {
          flush()
          out.push({ type: "image", alt: text.slice(index + 2, close) })
          index = end + 1
          continue
        }
      }
    }

    if (char === "[") {
      const close = closing(text, index)
      if (close !== -1 && text[close + 1] === "(") {
        const end = closing(text, close + 1, "()")
        if (end !== -1) {
          flush()
          const href =
            text
              .slice(close + 2, end)
              .trim()
              .split(/\s+/)[0] ?? ""
          out.push({
            type: "link",
            children: parseInline(text.slice(index + 1, close)),
            href,
          })
          index = end + 1
          continue
        }
      }
    }

    if (char === "~" && text[index + 1] === "~") {
      const end = text.indexOf("~~", index + 2)
      if (end > index + 2) {
        flush()
        out.push({
          type: "deleted",
          children: parseInline(text.slice(index + 2, end)),
        })
        index = end + 2
        continue
      }
    }

    if (char === "*" || char === "_") {
      const double = text[index + 1] === char
      const marker = double ? char + char : char
      // `snake_case` is not emphasis: an underscore inside a word stays text.
      const opensInWord = char === "_" && isWordChar(text[index - 1])
      const after = text[index + marker.length]
      if (!opensInWord && after !== undefined && after !== " ") {
        let end = text.indexOf(marker, index + marker.length)
        while (
          end !== -1 &&
          (text[end - 1] === " " ||
            (char === "_" && isWordChar(text[end + marker.length])) ||
            (!double && text[end + 1] === char))
        ) {
          end = text.indexOf(marker, end + 1)
        }
        if (end !== -1) {
          flush()
          const children = parseInline(text.slice(index + marker.length, end))
          out.push(
            double
              ? { type: "strong", children }
              : { type: "emphasis", children }
          )
          index = end + marker.length
          continue
        }
      }
      buffer += marker
      index += marker.length
      continue
    }

    buffer += char
    index += 1
  }

  flush()
  return out
}
