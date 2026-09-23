import { describe, expect, it } from "vitest"

import { CHAT_ROWS, rowsCaption } from "./assistant-tool-rows"

describe("what the chat grid says of its rows", () => {
  it("counts every row when the chat shows them all", () => {
    expect(rowsCaption(1, false)).toBe("1 row")
    expect(rowsCaption(CHAT_ROWS, false)).toBe(`${CHAT_ROWS} rows`)
  })

  it("says it shows the first rows only, and of how many", () => {
    expect(rowsCaption(1_200, false)).toBe("First 100 rows of 1,200")
  })

  it("says a result cut at the row limit is not whole", () => {
    expect(rowsCaption(1_200, true)).toBe(
      "First 100 rows of 1,200, cut at the row limit"
    )
  })
})
