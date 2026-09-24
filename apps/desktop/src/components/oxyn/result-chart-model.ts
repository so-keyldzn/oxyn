// What a result can be drawn as, decided from its columns and its first rows.
//
// Cells arrive formatted by Rust (`oxyn_data::format_cell`): a chart reads the
// text back, and only when it is unmistakably a number. A value it cannot read
// is never guessed at — the column is left out, and the chart says so.

import type { Cell, ResultColumn } from "@/lib/ipc/types"

/** Numeric columns one chart draws; more would be a legend, not a chart. */
export const CHART_MAX_SERIES = 4

/** Arrow's names, as `DataType`'s `Display` writes them. */
const NUMERIC = /^(U?Int(8|16|32|64)|Float(16|32|64)|Decimal(32|64|128|256))\b/
const TEMPORAL = /^(Date(32|64)|Timestamp)\b/
const CATEGORY = /^(Utf8|LargeUtf8|Utf8View|Boolean|Dictionary)\b/

export interface ChartPlan {
  /** Column of the horizontal axis. */
  axis: number
  /** A date axis draws a line; a category axis draws bars. */
  kind: "line" | "bar"
  /** Candidate numeric columns, in result order. */
  series: Array<number>
}

/**
 * A chart for these columns, or `null`: one date or category column for the
 * axis, and at least one numeric column. A date axis wins over a category one.
 */
export function chartPlan(columns: Array<ResultColumn>): ChartPlan | null {
  const temporal = columns.findIndex((column) => TEMPORAL.test(column.dataType))
  const category = columns.findIndex((column) => CATEGORY.test(column.dataType))
  const axis = temporal !== -1 ? temporal : category
  if (axis === -1) return null
  const series = columns
    .map((column, index) => ({ column, index }))
    .filter(
      ({ column, index }) => index !== axis && NUMERIC.test(column.dataType)
    )
    .map(({ index }) => index)
    .slice(0, CHART_MAX_SERIES)
  if (series.length === 0) return null
  return { axis, kind: temporal !== -1 ? "line" : "bar", series }
}

/** A plain decimal, possibly grouped by U+00A0 as Rust groups it. Nothing else. */
const NUMBER = /^[-+]?(\d+(\.\d*)?|\.\d+)([eE][-+]?\d+)?$/

/**
 * `oxyn_data::GROUP_SEPARATOR`, a no-break space between groups of three.
 * Spelled by its code: written out, it is invisible in the source.
 */
const GROUP_SEPARATOR = String.fromCharCode(0xa0)

/**
 * The number a cell spells, `null` for a SQL `NULL`, `undefined` when it is
 * not a number at all — a cut value, an unrenderable one, or any other text.
 */
export function cellNumber(cell: Cell): number | null | undefined {
  if (cell === null) return null
  if (typeof cell !== "string") return undefined
  const text = cell.replaceAll(GROUP_SEPARATOR, "")
  if (!NUMBER.test(text)) return undefined
  const value = Number(text)
  return Number.isFinite(value) ? value : undefined
}

/** The label of an axis cell, as Rust wrote it. */
function cellLabel(cell: Cell): string {
  if (cell === null) return "NULL"
  if (typeof cell === "string") return cell
  return "text" in cell ? cell.text : cell.unrenderable
}

export interface ChartData {
  kind: "line" | "bar"
  axisName: string
  /** Series drawn, each keyed `s0`, `s1`… in `rows`. */
  series: Array<{ key: string; name: string }>
  /** Numeric columns left out because a value was not a number. */
  skipped: Array<string>
  rows: Array<Record<string, string | number | null>>
}

/**
 * The rows, as a chart reads them, in the order the query returned them:
 * sorting here would draw an order the query did not ask for.
 */
export function chartData(
  plan: ChartPlan,
  columns: Array<ResultColumn>,
  rows: Array<Array<Cell>>
): ChartData | null {
  const readable = plan.series.filter((column) =>
    rows.every((row) => cellNumber(row[column] ?? null) !== undefined)
  )
  const series = readable
    .filter((column) =>
      rows.some((row) => typeof cellNumber(row[column] ?? null) === "number")
    )
    .map((column, index) => ({
      column,
      key: `s${index}`,
      name: columns[column]?.name ?? "",
    }))
  if (series.length === 0 || rows.length === 0) return null
  return {
    kind: plan.kind,
    axisName: columns[plan.axis]?.name ?? "",
    series: series.map(({ key, name }) => ({ key, name })),
    skipped: plan.series
      .filter((column) => !series.some((drawn) => drawn.column === column))
      .map((column) => columns[column]?.name ?? ""),
    rows: rows.map((row) => {
      const point: Record<string, string | number | null> = {
        axis: cellLabel(row[plan.axis] ?? null),
      }
      for (const { column, key } of series)
        point[key] = cellNumber(row[column] ?? null) ?? null
      return point
    }),
  }
}
