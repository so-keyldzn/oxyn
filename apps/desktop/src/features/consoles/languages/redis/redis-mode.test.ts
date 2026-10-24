import { StringStream } from "@codemirror/language"
import { describe, expect, it } from "vitest"

import { redisCommandParser } from "./redis-mode"

/** The non-blank tokens of `lines`, as `[text, style]`, in order. */
function tokens(...lines: Array<string>): Array<[string, string | null]> {
  const state = redisCommandParser.startState?.(2)
  if (!state) throw new Error("the parser has a start state")
  const out: Array<[string, string | null]> = []
  for (const line of lines) {
    const stream = new StringStream(line, 2, 2)
    while (!stream.eol()) {
      const style = redisCommandParser.token(stream, state)
      const text = stream.current()
      if (text.trim() !== "") out.push([text, style ?? null])
      stream.start = stream.pos
    }
  }
  return out
}

describe("the Redis editor mode", () => {
  it("reads the first argument as the command, whatever its name", () => {
    expect(tokens("HSET user:1 name 42")).toEqual([
      ["HSET", "keyword"],
      ["user:1", null],
      ["name", null],
      ["42", "number"],
    ])
    // No list: an unknown name is highlighted like any command.
    expect(tokens("JSON.SET k $ 1")[0]).toEqual(["JSON.SET", "keyword"])
  })

  it("keeps a quoted argument whole, escapes included", () => {
    expect(tokens("SET \"a \\\" b\" 'it\\'s'")).toEqual([
      ["SET", "keyword"],
      ['"a \\" b"', "string"],
      ["'it\\'s'", "string"],
    ])
  })

  it("carries an open quote across a line break, as the driver reads it", () => {
    expect(tokens('SET k "line', 'break" GET')).toEqual([
      ["SET", "keyword"],
      ["k", null],
      ['"line', "string"],
      ['break"', "string"],
      ["GET", null],
    ])
  })

  it("does not read a key that starts with digits as a number", () => {
    expect(tokens("GET 42abc")[1]).toEqual(["42abc", null])
  })
})
