import { describe, expect, it } from "vitest"

import { MAX_COPY_ROWS, copyText, inRange, rangeOf } from "./grid-selection"
import type { Cell, ResultColumn } from "@/lib/ipc/types"

const columns: Array<ResultColumn> = [
  { name: "id", dataType: "Int64", nullable: false },
  { name: "note", dataType: "Utf8", nullable: true },
]

describe("grid selection", () => {
  it("normalizes a range whatever the direction of the drag", () => {
    const range = rangeOf({ row: 9, column: 1 }, { row: 2, column: 0 })
    expect(range).toEqual({ top: 2, bottom: 9, left: 0, right: 1 })
    expect(inRange(range, 5, 1)).toBe(true)
    expect(inRange(range, 10, 1)).toBe(false)
    expect(rangeOf(null, { row: 0, column: 0 })).toBeNull()
  })

  it("copies what is shown, as tab-separated text", () => {
    const rows: Array<Array<Cell>> = [
      ["1", null],
      ["2", 'says "hi"\tthen\nleaves'],
    ]
    const copied = copyText(
      rows,
      columns,
      { top: 0, bottom: 1, left: 0, right: 1 },
      true,
      [0, 1]
    )
    expect(copied).toEqual({
      ok: true,
      text: 'id\tnote\n1\t\n2\t"says ""hi""\tthen\nleaves"',
      rows: 2,
      cells: 4,
    })
  })

  it("leaves a hidden column out of the copy", () => {
    const rows: Array<Array<Cell>> = [["1", { text: "Long no", fullBytes: 9 }]]
    const copied = copyText(
      rows,
      columns,
      { top: 0, bottom: 0, left: 0, right: 0 },
      true,
      [0]
    )
    // The hidden value is truncated: it is not even looked at.
    expect(copied).toEqual({ ok: true, text: "id\n1", rows: 1, cells: 1 })
  })

  it("copies moved columns where they stand, by their own index", () => {
    const rows: Array<Array<Cell>> = [["1", "first"]]
    const copied = copyText(
      rows,
      columns,
      { top: 0, bottom: 0, left: 0, right: 1 },
      true,
      [1, 0]
    )
    expect(copied).toEqual({
      ok: true,
      text: "note\tid\nfirst\t1",
      rows: 1,
      cells: 2,
    })
  })

  it("refuses a truncated value rather than pasting half of it", () => {
    const rows: Array<Array<Cell>> = [
      ["1", { text: "Long note", fullBytes: 48_213 }],
    ]
    const copied = copyText(
      rows,
      columns,
      { top: 0, bottom: 0, left: 0, right: 1 },
      false,
      [0, 1]
    )
    expect(copied.ok).toBe(false)
    if (!copied.ok) expect(copied.reason).toMatch(/truncated/)
  })

  it("refuses more rows than one page", () => {
    const copied = copyText(
      [],
      columns,
      { top: 0, bottom: MAX_COPY_ROWS, left: 0, right: 0 },
      false,
      [0]
    )
    expect(copied.ok).toBe(false)
  })

  it("keeps a hostile value as text", () => {
    const hostile = '"; DROP TABLE audit; --'
    const copied = copyText(
      [[hostile, "x"]],
      columns,
      { top: 0, bottom: 0, left: 0, right: 0 },
      false,
      [0, 1]
    )
    expect(copied).toMatchObject({
      ok: true,
      text: '"""; DROP TABLE audit; --"',
    })
  })
})
