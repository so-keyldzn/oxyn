import { cellText } from "@/components/oxyn/cell-value"
import type { Cell, ResultColumn } from "@/lib/ipc/types"

/** A cell position: global row, Arrow column index. */
export interface GridPosition {
  row: number
  column: number
}

/** An inclusive rectangle of cells. */
export interface GridRange {
  top: number
  bottom: number
  left: number
  right: number
}

/**
 * Rows one copy may carry. It is one page call (`MAX_PAGE_ROWS`): past that,
 * the export writes the whole result without holding it in the webview (I-06).
 */
export const MAX_COPY_ROWS = 2000

export function rangeOf(
  anchor: GridPosition | null,
  focus: GridPosition | null
): GridRange | null {
  if (!anchor || !focus) return null
  return {
    top: Math.min(anchor.row, focus.row),
    bottom: Math.max(anchor.row, focus.row),
    left: Math.min(anchor.column, focus.column),
    right: Math.max(anchor.column, focus.column),
  }
}

export function inRange(range: GridRange | null, row: number, column: number) {
  return (
    range !== null &&
    row >= range.top &&
    row <= range.bottom &&
    column >= range.left &&
    column <= range.right
  )
}

export function rangeRows(range: GridRange) {
  return range.bottom - range.top + 1
}

export type CopyText =
  | { ok: true; text: string; rows: number; cells: number }
  | { ok: false; reason: string }

/** Quotes a field the way spreadsheets read tab-separated text back. */
function field(text: string) {
  return /[\t\n\r"]/.test(text) ? `"${text.replaceAll('"', '""')}"` : text
}

/**
 * The tab-separated text of a range, **as the grid shows it**.
 *
 * Refused rather than shortened when a value in the range is truncated or
 * cannot be rendered: pasting « Long note Long no… » into a ticket is a wrong
 * value that looks right. The full value is one `Inspect full value` away, and
 * the export writes every value whole. `NULL` copies as an empty field, the
 * spreadsheet convention.
 */
export function copyText(
  rows: ReadonlyArray<ReadonlyArray<Cell>>,
  columns: ReadonlyArray<ResultColumn>,
  range: GridRange,
  withHeaders: boolean
): CopyText {
  if (rangeRows(range) > MAX_COPY_ROWS) {
    return {
      ok: false,
      reason: `Not copied: at most ${MAX_COPY_ROWS.toLocaleString("en-US")} rows are copied at once. Export the result to keep more.`,
    }
  }
  if (rows.length < rangeRows(range)) {
    return {
      ok: false,
      reason: "Not copied: some selected rows are not loaded.",
    }
  }
  let truncated = 0
  let unrenderable = 0
  const lines: Array<string> = []
  if (withHeaders) {
    const names: Array<string> = []
    for (let column = range.left; column <= range.right; column++) {
      names.push(field(columns[column]?.name ?? ""))
    }
    lines.push(names.join("\t"))
  }
  for (const row of rows) {
    const values: Array<string> = []
    for (let column = range.left; column <= range.right; column++) {
      const cell = row[column] ?? null
      if (cell !== null && typeof cell === "object") {
        if ("text" in cell) truncated++
        else unrenderable++
      }
      values.push(field(cellText(cell)))
    }
    lines.push(values.join("\t"))
  }
  if (truncated > 0) {
    return {
      ok: false,
      reason: `Not copied: ${truncated} selected ${truncated === 1 ? "value is" : "values are"} truncated in the grid. Inspect the full value, or export the result.`,
    }
  }
  if (unrenderable > 0) {
    return {
      ok: false,
      reason: `Not copied: ${unrenderable} selected ${unrenderable === 1 ? "value" : "values"} cannot be rendered as text.`,
    }
  }
  const width = range.right - range.left + 1
  return {
    ok: true,
    text: lines.join("\n"),
    rows: rows.length,
    cells: rows.length * width,
  }
}
