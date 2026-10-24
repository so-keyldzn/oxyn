import type { FetchPage } from "@/components/oxyn/result-grid"

/**
 * Rows of one chat page. The rest of the result is reached page by page, read
 * again from the buffer the executor retained, or one click away in a result
 * tab: a conversation is read, not scrolled through a million rows.
 */
export const CHAT_ROWS = 100

/** One page of the retained result: rows `[start, end)`, in buffer order. */
export interface PageRange {
  start: number
  end: number
}

/** A count from the boundary, as a whole number of rows, never negative. */
function whole(value: number) {
  return Number.isFinite(value) ? Math.max(0, Math.floor(value)) : 0
}

/** Pages a result of `rows` rows spans; none when it has no rows. */
export function pageCount(rows: number, size = CHAT_ROWS) {
  return Math.ceil(whole(rows) / Math.max(1, whole(size)))
}

/**
 * The page actually shown for an asked index: never before the first, never
 * past the last — a result that shrank under the view lands on its last page.
 */
export function clampPage(page: number, rows: number, size = CHAT_ROWS) {
  return Math.min(whole(page), Math.max(0, pageCount(rows, size) - 1))
}

/** The rows a page holds; the last page may be short. */
export function pageRange(
  page: number,
  rows: number,
  size = CHAT_ROWS
): PageRange {
  const total = whole(rows)
  const step = Math.max(1, whole(size))
  const start = Math.min(total, clampPage(page, total, step) * step)
  return { start, end: Math.min(total, start + step) }
}

/** The interface speaks English: its numbers too, as in the approval dialog. */
const count = new Intl.NumberFormat("en-US")

function counted(rows: number) {
  return `${count.format(rows)} ${rows === 1 ? "row" : "rows"}`
}

/** « 101–200 of 250,000 »: the rows of a page, numbered from one. */
export function rangeText(range: PageRange, rows: number) {
  return `${count.format(range.start + 1)}–${count.format(range.end)} of ${count.format(whole(rows))}`
}

/** What the footer says of the rows shown. Pure, so it is tested. */
export function rowsCaption(
  range: PageRange,
  rows: number,
  truncated: boolean
) {
  const head =
    pageCount(rows) > 1
      ? `Rows ${rangeText(range, rows)}`
      : counted(whole(rows))
  return truncated ? `${head}, cut at the row limit` : head
}

/**
 * Where a read the grid asks inside its page lands in the retained result.
 *
 * The grid counts from its own first row; the backend from the result's. The
 * window is clamped to the page, hence to the rows retained: whatever the grid
 * asks, no request reaches past `range.end`.
 */
export function pageWindow(range: PageRange, offset: number, limit: number) {
  const start = Math.min(range.end, range.start + whole(offset))
  return { offset: start, limit: Math.min(whole(limit), range.end - start) }
}

/**
 * The page as a result of its own, for the grid and the chart: offsets go
 * through `pageWindow`, and an answer is told back in the page's own rows, so
 * a page never reads as holding more than it does.
 */
export function pageFetch(fetchPage: FetchPage, range: PageRange): FetchPage {
  return async (offset, limit) => {
    const window = pageWindow(range, offset, limit)
    const answer = await fetchPage(window.offset, window.limit)
    if (answer.type !== "page") return answer
    return {
      ...answer,
      offset: answer.offset - range.start,
      totalRows: Math.min(
        range.end - range.start,
        Math.max(0, answer.totalRows - range.start)
      ),
    }
  }
}
