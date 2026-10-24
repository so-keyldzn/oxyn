import { StringStream } from "@codemirror/language"
import { describe, expect, it } from "vitest"

import { searchDslParser } from "./search-dsl-mode"

/** The non-blank tokens of `lines`, as `[text, style]`, in order. */
function tokens(...lines: Array<string>): Array<[string, string | null]> {
  const state = searchDslParser.startState?.(2)
  if (!state) throw new Error("the parser has a start state")
  const out: Array<[string, string | null]> = []
  for (const line of lines) {
    const stream = new StringStream(line, 2, 2)
    while (!stream.eol()) {
      const style = searchDslParser.token(stream, state)
      const text = stream.current()
      if (text.trim() !== "") out.push([text, style ?? null])
      stream.start = stream.pos
    }
  }
  return out
}

describe("the Search DSL editor mode", () => {
  it("reads the method, the API segments and the targets of a request line", () => {
    expect(tokens("POST /logs-*/_search?size=0")).toEqual([
      ["POST", "keyword"],
      ["/", "punctuation"],
      ["logs-*", "url"],
      ["/", "punctuation"],
      ["_search", "keyword"],
      ["?size=0", "attributeName"],
    ])
    // Kibana's form without the leading slash, in any case.
    expect(tokens("get _cat/indices")[0]).toEqual(["get", "keyword"])
    expect(tokens("get _cat/indices")[1]).toEqual(["_cat", "keyword"])
  })

  it("tells an object's keys from its string values in the body", () => {
    expect(
      tokens(
        "GET /_search",
        '{"query": {"term": {"status": "500"}}, "size": 10, "x": null}'
      )
    ).toEqual([
      ["GET", "keyword"],
      ["/", "punctuation"],
      ["_search", "keyword"],
      ["{", "punctuation"],
      ['"query"', "propertyName"],
      [":", "punctuation"],
      ["{", "punctuation"],
      ['"term"', "propertyName"],
      [":", "punctuation"],
      ["{", "punctuation"],
      ['"status"', "propertyName"],
      [":", "punctuation"],
      ['"500"', "string"],
      ["}", "punctuation"],
      ["}", "punctuation"],
      [",", "punctuation"],
      ['"size"', "propertyName"],
      [":", "punctuation"],
      ["10", "number"],
      [",", "punctuation"],
      ['"x"', "propertyName"],
      [":", "punctuation"],
      ["null", "null"],
      ["}", "punctuation"],
    ])
  })

  it("does not take a body line that names a method for a new request", () => {
    // Inside a string, `GET` is text; a body line is only a body line.
    expect(tokens('{"q": "a', 'GET b"}')).toEqual([
      ["{", "punctuation"],
      ['"q"', "propertyName"],
      [":", "punctuation"],
      ['"a', "string"],
      ['GET b"', "string"],
      ["}", "punctuation"],
    ])
  })

  it("reads a comment line as a comment", () => {
    expect(tokens("# the slow ones", "GET /_tasks")[0]).toEqual([
      "# the slow ones",
      "comment",
    ])
  })
})
