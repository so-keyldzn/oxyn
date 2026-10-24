import { describe, expect, it, vi } from "vitest"

import {
  CHAT_ROWS,
  clampPage,
  pageCount,
  pageFetch,
  pageRange,
  pageWindow,
  rowsCaption,
} from "./assistant-tool-rows-pages"
import type { FetchPage } from "./result-grid"

describe("the pages of a retained result", () => {
  it("counts no page for no rows, one up to a full page, two past it", () => {
    expect(pageCount(0)).toBe(0)
    expect(pageCount(1)).toBe(1)
    expect(pageCount(CHAT_ROWS)).toBe(1)
    expect(pageCount(CHAT_ROWS + 1)).toBe(2)
    expect(pageCount(250_000)).toBe(2_500)
  })

  it("gives the first, a middle and a short last page", () => {
    expect(pageRange(0, 250)).toEqual({ start: 0, end: 100 })
    expect(pageRange(1, 250)).toEqual({ start: 100, end: 200 })
    expect(pageRange(2, 250)).toEqual({ start: 200, end: 250 })
  })

  it("holds every row of a result that fits one page", () => {
    expect(pageRange(0, CHAT_ROWS)).toEqual({ start: 0, end: CHAT_ROWS })
    expect(pageRange(0, 3)).toEqual({ start: 0, end: 3 })
  })

  it("puts row 101 alone on the second page", () => {
    expect(pageRange(1, CHAT_ROWS + 1)).toEqual({ start: 100, end: 101 })
  })

  it("holds nothing for a result with no rows", () => {
    expect(pageRange(0, 0)).toEqual({ start: 0, end: 0 })
    expect(pageRange(4, 0)).toEqual({ start: 0, end: 0 })
  })

  it("never stands past the last page, or before the first", () => {
    expect(clampPage(7, 250)).toBe(2)
    expect(clampPage(-3, 250)).toBe(0)
    expect(clampPage(Number.NaN, 250)).toBe(0)
    expect(clampPage(2, 0)).toBe(0)
  })

  it("lands on the last page when the rows shrink below the one shown", () => {
    expect(clampPage(9, 120)).toBe(1)
    expect(pageRange(9, 120)).toEqual({ start: 100, end: 120 })
  })
})

describe("what the chat grid says of its rows", () => {
  it("counts every row when the chat shows them all", () => {
    expect(rowsCaption(pageRange(0, 1), 1, false)).toBe("1 row")
    expect(rowsCaption(pageRange(0, CHAT_ROWS), CHAT_ROWS, false)).toBe(
      `${CHAT_ROWS} rows`
    )
  })

  it("names the rows of the page shown, and of how many", () => {
    expect(rowsCaption(pageRange(0, 250_000), 250_000, false)).toBe(
      "Rows 1–100 of 250,000"
    )
    expect(rowsCaption(pageRange(1, 250_000), 250_000, false)).toBe(
      "Rows 101–200 of 250,000"
    )
    expect(rowsCaption(pageRange(1, 101), 101, false)).toBe(
      "Rows 101–101 of 101"
    )
  })

  it("says a result cut at the row limit is not whole", () => {
    expect(rowsCaption(pageRange(2, 1_200), 1_200, true)).toBe(
      "Rows 201–300 of 1,200, cut at the row limit"
    )
  })
})

describe("a read the grid asks inside its page", () => {
  const middle = pageRange(1, 250)
  const last = pageRange(2, 250)

  it("starts from the page's first row", () => {
    expect(pageWindow(middle, 0, 100)).toEqual({ offset: 100, limit: 100 })
    expect(pageWindow(middle, 40, 20)).toEqual({ offset: 140, limit: 20 })
  })

  it("never reaches past the page, hence past the rows retained", () => {
    expect(pageWindow(last, 0, 200)).toEqual({ offset: 200, limit: 50 })
    expect(pageWindow(last, 49, 2_000)).toEqual({ offset: 249, limit: 1 })
    expect(pageWindow(last, 50, 10)).toEqual({ offset: 250, limit: 0 })
    expect(pageWindow(last, 1e12, 1e12)).toEqual({ offset: 250, limit: 0 })
    expect(pageWindow(last, -5, Number.POSITIVE_INFINITY)).toEqual({
      offset: 200,
      limit: 0,
    })
  })

  it("asks the backend in the result's rows and answers in the page's", async () => {
    const backend = vi.fn<FetchPage>((offset, limit) =>
      Promise.resolve({
        type: "page",
        offset,
        rows: Array.from({ length: limit }, (_, row) => [String(offset + row)]),
        totalRows: 250,
        complete: true,
      })
    )
    const answer = await pageFetch(backend, last)(10, 500)
    expect(backend).toHaveBeenCalledExactlyOnceWith(210, 40)
    expect(answer).toMatchObject({ type: "page", offset: 10, totalRows: 50 })
    expect(answer.type === "page" && answer.rows[0]).toEqual(["210"])
  })

  it("passes an expired answer through as it came", async () => {
    const backend: FetchPage = () => Promise.resolve({ type: "expired" })
    await expect(pageFetch(backend, middle)(0, 100)).resolves.toEqual({
      type: "expired",
    })
  })
})
