// Where the cursor stands in a JSON text being typed, for the completion.
//
// Read by hand rather than from the Lezer tree: the text is unfinished by
// nature — an unclosed string, a key without its colon, a bare `$g` — and what
// matters is the chain of keys from the root to the cursor, which a tolerant
// left-to-right scan gives directly. Nothing here is sent: the server reads the
// text through `oxyn-query`, and this only chooses what to suggest.

/** One step from the root: a key of an object, or an index of an array. */
export type Step = string | number

/** What the cursor completes. */
export interface Slot {
  /** A property name, or a value. */
  role: "key" | "value"
  /**
   * Where the replaced text starts: after the opening quote inside a string,
   * at the first character of a bare word otherwise.
   */
  from: number
  /** Where it ends: before the closing quote, or at the end of the word. */
  to: number
  /** Inside a string: what is inserted is its content, not a literal. */
  quoted: boolean
  /** The string has its closing quote on its line. */
  closed: boolean
  /** What is typed between `from` and the cursor, as written. */
  typed: string
  /** The keys and indexes leading to the container the slot is in. */
  route: Array<Step>
  /**
   * The first key of each object along `route`, its container last: a
   * command names itself with its first key. `null` for an array, or an
   * object whose first key comes after the cursor.
   */
  heads: Array<string | null>
  /** For a value in an object: the key it is the value of. */
  key: string | null
  /** How many keys of the container come before the slot. */
  position: number
  /** The keys the container holds, the slot's own excepted. */
  siblings: Array<string>
}

/** What the scan found: the slot, and the root's keys with their strings. */
export interface Reading {
  slot: Slot | null
  /** The root object's keys, in text order. */
  rootKeys: Array<string>
  /** The root keys whose value is a string, with that string. */
  rootStrings: Map<string, string>
}

interface ObjectFrame {
  kind: "object"
  expect: "key" | "colon" | "value" | "comma"
  /** The key whose value is being read, once read. */
  key: string | null
  keys: Array<string>
}

interface ArrayFrame {
  kind: "array"
  expect: "value" | "comma"
  index: number
}

type Frame = ObjectFrame | ArrayFrame

/** Where the cursor was found while scanning. */
interface Found {
  slot: Omit<Slot, "siblings">
  /** The object the slot is in: its keys are known once it is read whole. */
  frame: ObjectFrame | null
  /** The index the slot's own key takes among them, when it is a key. */
  own: number | null
}

const DELIMITER = /[\s{}[\],:"]/

/** The string a JSON literal holds, or its raw content when malformed. */
function decode(literal: string): string {
  try {
    const value: unknown = JSON.parse(literal)
    return typeof value === "string" ? value : literal.slice(1, -1)
  } catch {
    return literal.slice(1, -1)
  }
}

/** The role a token at the current place would play, or null. */
function roleOf(frame: Frame | undefined): "key" | "value" | null {
  if (frame === undefined) return null
  if (frame.kind === "array") return frame.expect === "value" ? "value" : null
  if (frame.expect === "key") return "key"
  if (frame.expect === "value") return "value"
  return null
}

/**
 * Where the string opened at `start` ends: after its closing quote, or — not
 * closed — at the end of its line, so that the next lines keep their meaning
 * while a key is being typed.
 */
function stringEnd(text: string, start: number) {
  for (let at = start + 1; at < text.length; at++) {
    const char = text[at]
    if (char === "\\") at++
    else if (char === '"') return { end: at + 1, closed: true }
    else if (char === "\n") return { end: at, closed: false }
  }
  return { end: text.length, closed: false }
}

/**
 * Reads the whole `text` and reports what the cursor at `pos` completes.
 *
 * Tolerant: an unclosed string ends with its line, a closing bracket that
 * matches nothing is skipped. Linear in the text.
 */
export function readJson(text: string, pos: number): Reading {
  const stack: Array<Frame> = []
  const rootKeys: Array<string> = []
  const rootStrings = new Map<string, string>()
  // `undefined` until the cursor is passed, `null` where nothing completes.
  let found: Found | null | undefined

  const top = () => stack[stack.length - 1]
  const take = (
    from: number,
    to: number,
    token: { quoted: boolean; closed: boolean } | null
  ): Found | null => {
    const frame = top()
    const role = roleOf(frame)
    if (role === null) return null
    const objectFrame = frame?.kind === "object" ? frame : null
    return {
      slot: {
        role,
        from,
        to,
        quoted: token?.quoted ?? false,
        closed: token?.closed ?? false,
        typed: text.slice(from, Math.max(from, pos)),
        route: stack
          .slice(0, -1)
          .map((step) =>
            step.kind === "array" ? step.index : (step.key ?? "")
          ),
        heads: stack.map((step) =>
          step.kind === "object" ? (step.keys[0] ?? null) : null
        ),
        key: role === "value" ? (objectFrame?.key ?? null) : null,
        position: objectFrame?.keys.length ?? 0,
      },
      frame: objectFrame,
      // Inside a token, the key that token becomes is the slot's own.
      own:
        objectFrame && role === "key" && token ? objectFrame.keys.length : null,
    }
  }
  const keyRead = (frame: ObjectFrame, key: string) => {
    frame.key = key
    frame.keys.push(key)
    if (stack.length === 1) rootKeys.push(key)
    frame.expect = "colon"
  }
  // A value was read in the current container: a comma comes next.
  const valueRead = (value: string | null) => {
    const frame = top()
    if (frame === undefined) return
    if (frame.kind === "object" && stack.length === 1 && frame.key !== null)
      if (value !== null) rootStrings.set(frame.key, value)
    frame.expect = "comma"
  }

  let at = 0
  while (at < text.length) {
    const char = text[at] ?? ""
    if (/\s/.test(char)) {
      at++
      continue
    }
    // The cursor sits between two tokens, before this one.
    if (found === undefined && at >= pos) found = take(pos, pos, null)
    const frame = top()
    if (char === '"') {
      const { end, closed } = stringEnd(text, at)
      const contentEnd = closed ? end - 1 : end
      if (found === undefined && pos > at && pos <= contentEnd)
        found = take(at + 1, contentEnd, { quoted: true, closed })
      const literal = text.slice(at, end)
      if (frame?.kind === "object" && frame.expect === "key")
        keyRead(frame, closed ? decode(literal) : literal.slice(1))
      else valueRead(closed ? decode(literal) : null)
      at = end
    } else if (char === "{") {
      stack.push({ kind: "object", expect: "key", key: null, keys: [] })
      at++
    } else if (char === "[") {
      stack.push({ kind: "array", expect: "value", index: 0 })
      at++
    } else if (char === "}" || char === "]") {
      if (frame && (frame.kind === "object") === (char === "}")) {
        stack.pop()
        valueRead(null)
      }
      at++
    } else if (char === ":") {
      if (frame?.kind === "object" && frame.expect === "colon")
        frame.expect = "value"
      at++
    } else if (char === ",") {
      if (frame?.kind === "object") {
        frame.expect = "key"
        frame.key = null
      } else if (frame?.kind === "array") {
        frame.expect = "value"
        frame.index++
      }
      at++
    } else {
      // A bare word: a number, true, false, null — or a name typed without
      // its quotes yet.
      let end = at
      while (end < text.length && !DELIMITER.test(text[end] ?? "")) end++
      if (found === undefined && pos > at && pos <= end)
        found = take(at, end, { quoted: false, closed: false })
      if (frame?.kind === "object" && frame.expect === "key")
        keyRead(frame, text.slice(at, end))
      else valueRead(null)
      at = end
    }
  }
  if (found === undefined && pos <= text.length) found = take(pos, pos, null)

  if (!found) return { slot: null, rootKeys, rootStrings }
  const { slot, frame, own } = found
  const siblings = (frame?.keys ?? []).filter((_, index) => index !== own)
  return { slot: { ...slot, siblings }, rootKeys, rootStrings }
}
