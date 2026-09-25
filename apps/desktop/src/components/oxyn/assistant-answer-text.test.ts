import { describe, expect, it } from "vitest"

import { answerReadableText } from "./assistant-answer-text"

describe("the readable text of an answer", () => {
  it("drops the markup and keeps what is shown", () => {
    expect(
      answerReadableText(
        "## Result\n\nThere are **1,204** active `clients`.\n\n- one\n- two\n\n1. first\n2. second"
      )
    ).toBe(
      "Result\n\nThere are 1,204 active clients.\n\n- one\n- two\n\n1. first\n2. second"
    )
  })

  it("keeps a link's address and a code block's text", () => {
    expect(
      answerReadableText(
        "See [the docs](https://example.com).\n\n```sql\nSELECT 1;\n```"
      )
    ).toBe("See the docs (https://example.com).\n\nSELECT 1;")
  })

  it("writes a table as tab-separated rows", () => {
    expect(answerReadableText("| a | b |\n| --- | --- |\n| 1 | 2 |")).toBe(
      "a\tb\n1\t2"
    )
  })

  it("keeps raw HTML as the text it is", () => {
    expect(answerReadableText("<b>bold</b>")).toBe("<b>bold</b>")
  })
})
