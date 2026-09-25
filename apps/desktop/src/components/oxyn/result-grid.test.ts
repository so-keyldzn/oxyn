import { describe, expect, it } from "vitest"

import { footerSummary, formatElapsed } from "./result-footer"
import {
  MAX_AUTO_WIDTH,
  MIN_COLUMN_WIDTH,
  PAGE_SIZE,
  estimateWidth,
  fillPage,
  isNumericType,
  pageSizeFor,
  pagesFor,
  shownColumns,
  stepColumn,
} from "./result-grid"
import type { PageAnswer } from "./result-grid"
import { statementPreview } from "./result-panel"
import type { Cell } from "@/lib/ipc/types"

describe("hidden columns", () => {
  it("draws the other columns under their own Arrow index", () => {
    expect(shownColumns(4)).toEqual([0, 1, 2, 3])
    expect(shownColumns(4, new Set([1, 3]))).toEqual([0, 2])
  })

  it("steps over a hidden column and stops at the ends", () => {
    const shown = [0, 2, 3]
    expect(stepColumn(shown, 0, 1)).toBe(2)
    expect(stepColumn(shown, 2, -1)).toBe(0)
    expect(stepColumn(shown, 3, 1)).toBe(3)
    expect(stepColumn(shown, 0, -1)).toBe(0)
  })

  it("moves off a column just hidden to the nearest shown one", () => {
    const shown = [0, 2, 3]
    expect(stepColumn(shown, 1, 0)).toBe(2)
    expect(stepColumn(shown, 1, 1)).toBe(2)
    expect(stepColumn(shown, 1, -1)).toBe(0)
    expect(stepColumn([0, 1], 5, 0)).toBe(1)
    expect(stepColumn([0, 1], 5, -1)).toBe(1)
  })
})

describe("moved columns", () => {
  it("are drawn in the order moved to, still by Arrow index", () => {
    expect(shownColumns(4, undefined, [2, 0, 3, 1])).toEqual([2, 0, 3, 1])
    expect(shownColumns(4, new Set([0]), [2, 0, 3, 1])).toEqual([2, 3, 1])
    // An order for another width of result is not applied to this one.
    expect(shownColumns(3, undefined, [1, 0])).toEqual([0, 1, 2])
  })

  it("step in the order drawn", () => {
    const shown = [2, 0, 3]
    expect(stepColumn(shown, 2, 1)).toBe(0)
    expect(stepColumn(shown, 0, 1)).toBe(3)
    expect(stepColumn(shown, 3, -1)).toBe(0)
    expect(stepColumn(shown, 2, -1)).toBe(2)
    // Hidden column 1: the shown one of the next index, 2, is a step right.
    expect(stepColumn(shown, 1, 0)).toBe(2)
  })
})

describe("pagesFor", () => {
  it("asks only for the pages the viewport touches", () => {
    expect(pagesFor(0, 40)).toEqual([0])
    expect(pagesFor(190, 230)).toEqual([0, 1])
    expect(pagesFor(15, 25, 20)).toEqual([0, 1])
  })

  it("jumps straight to a far page without loading what lies before", () => {
    const row = 40_000_000
    expect(pagesFor(row, row + 40)).toEqual([Math.floor(row / PAGE_SIZE)])
  })

  it("asks for nothing when nothing is visible", () => {
    expect(pagesFor(0, -1)).toEqual([])
  })
})

describe("pageSizeFor", () => {
  it("keeps a page near twenty thousand cells, between 20 and 200 rows", () => {
    expect(pageSizeFor(7)).toBe(200)
    expect(pageSizeFor(400)).toBe(50)
    expect(pageSizeFor(1000)).toBe(20)
    expect(pageSizeFor(50_000)).toBe(20)
    expect(pageSizeFor(0)).toBe(200)
  })
})

describe("isNumericType", () => {
  it("right-aligns integers, floats and decimals only", () => {
    for (const type of ["Int64", "UInt8", "Float64", "Decimal128(12, 2)"]) {
      expect(isNumericType(type)).toBe(true)
    }
    for (const type of [
      "Utf8",
      "Interval(DayTime)",
      "Timestamp(Microsecond, None)",
      "Binary",
    ]) {
      expect(isNumericType(type)).toBe(false)
    }
  })
})

describe("estimateWidth", () => {
  const column = { name: "id", dataType: "Int64", nullable: false }

  it("fits the header when no row is loaded", () => {
    expect(estimateWidth(column, [])).toBeGreaterThanOrEqual(MIN_COLUMN_WIDTH)
  })

  it("widens for a timestamp and caps a long text", () => {
    const stamp = estimateWidth(column, ["2026-01-01T00:03:00.000000+00:00"])
    expect(stamp).toBeGreaterThan(200)
    const long: Array<Cell> = ["x".repeat(10_000)]
    expect(estimateWidth(column, long)).toBe(MAX_AUTO_WIDTH)
  })
})

describe("fillPage", () => {
  const rowsOf = (from: number, count: number) =>
    Array.from({ length: count }, (_, index) => [String(from + index)])

  it("asks again from where a page cut by the byte budget stopped", async () => {
    const calls: Array<[number, number]> = []
    const answer = await fillPage(
      (offset, limit) => {
        calls.push([offset, limit])
        return Promise.resolve({
          type: "page" as const,
          offset,
          rows: rowsOf(offset, Math.min(limit, 30)),
          totalRows: 1000,
          complete: true,
        })
      },
      100,
      80
    )
    expect(calls).toEqual([
      [100, 80],
      [130, 50],
      [160, 20],
    ])
    expect("rows" in answer && answer.rows.length).toBe(80)
    expect("offset" in answer && answer.offset).toBe(100)
  })

  it("stops at the end of a result that holds fewer rows", async () => {
    let calls = 0
    const answer = await fillPage(
      (offset) => {
        calls++
        return Promise.resolve({
          type: "page" as const,
          offset,
          rows: rowsOf(offset, 5),
          totalRows: 5,
          complete: false,
        })
      },
      0,
      200
    )
    expect(calls).toBe(1)
    expect("rows" in answer && answer.rows.length).toBe(5)
  })

  it("passes an expired answer through", async () => {
    const expired: PageAnswer = { type: "expired" }
    let first = true
    const answer = await fillPage(
      (offset) => {
        if (first) {
          first = false
          return Promise.resolve({
            type: "page" as const,
            offset,
            rows: rowsOf(offset, 1),
            totalRows: 10,
            complete: true,
          })
        }
        return Promise.resolve(expired)
      },
      0,
      10
    )
    expect(answer).toEqual(expired)
  })
})

describe("footerSummary", () => {
  it("says what is shown and how long it took", () => {
    expect(
      footerSummary({
        status: "done",
        rows: 1234,
        elapsedMs: 840,
        truncated: false,
        cancelled: false,
      })
    ).toEqual({ text: "1,234 rows shown · 840 ms", warning: false })
  })

  it("warns that a truncated or cancelled result is partial", () => {
    const partial = footerSummary({
      status: "done",
      rows: 1,
      truncated: true,
      cancelled: false,
    })
    expect(partial.warning).toBe(true)
    expect(partial.text).toBe("Truncated · 1 row shown, not the whole result")
    expect(
      footerSummary({
        status: "done",
        rows: 5,
        truncated: false,
        cancelled: true,
      }).text
    ).toMatch(/^Cancelled/)
  })

  it("counts received rows while running", () => {
    expect(
      footerSummary({ status: "running", rows: 12_000, serverCancel: true })
        .text
    ).toBe("12,000 rows received · running")
  })
})

describe("formatElapsed", () => {
  it("picks a readable unit", () => {
    expect(formatElapsed(12)).toBe("12 ms")
    expect(formatElapsed(12_400)).toBe("12.4 s")
    expect(formatElapsed(185_000)).toBe("3 min 05 s")
  })
})

describe("statementPreview", () => {
  it("keeps the first line, on one line, bounded", () => {
    expect(statementPreview("  SELECT *\n  FROM invoice")).toBe("SELECT *…")
    expect(statementPreview("SELECT   1")).toBe("SELECT 1")
    expect(statementPreview("x".repeat(200))).toHaveLength(81)
    expect(statementPreview("")).toBe("")
  })
})
