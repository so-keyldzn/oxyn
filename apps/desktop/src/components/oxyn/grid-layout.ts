// The layout a user gives a result's columns — the widths they dragged, the
// order they moved them into, and the widths the first rows suggested — kept
// apart from the grid that draws it. A view that remounts its grid while it
// still shows the same result, as the chat does for each of its pages (UX-SPEC
// « The rows of an agent query are read under its call »), hands the grid one
// of these so the layout outlives the remount. A display state, like the
// order: the rows, the SQL and an export never see it.

import type { ResultColumn } from "@/lib/ipc/types"

/** Widths estimated from the first rows, at the size cells are drawn at. */
export interface EstimatedWidths {
  charWidth: number
  nullText: string
  widths: Array<number>
}

export interface GridLayout {
  /** Arrow index to the width the user gave it. */
  resized: Readonly<Record<number, number>>
  estimated: EstimatedWidths | null
  /** `null` until the user moves a column. */
  order: ReadonlyArray<number> | null
}

/**
 * What the layout is bound to besides the result: a width or a position
 * belongs to a column, and another schema brings other columns under the same
 * indexes.
 */
export function schemaOf(columns: ReadonlyArray<ResultColumn>) {
  return JSON.stringify(columns.map((column) => [column.name, column.dataType]))
}

export interface GridLayoutStore {
  /** The layout kept for this result and these columns, or `null`. */
  read: (key: string, columns: ReadonlyArray<ResultColumn>) => GridLayout | null
  write: (
    key: string,
    columns: ReadonlyArray<ResultColumn>,
    layout: GridLayout
  ) => void
}

/**
 * One result's layout, and one only: writing for another result, or for the
 * same one with other columns, forgets the previous layout. The store lives as
 * long as the view that created it — nothing global to evict once a result is
 * released.
 */
export function gridLayoutStore(): GridLayoutStore {
  let kept: { key: string; schema: string; layout: GridLayout } | null = null
  return {
    read: (key, columns) =>
      kept?.key === key && kept.schema === schemaOf(columns)
        ? kept.layout
        : null,
    write: (key, columns, layout) => {
      kept = { key, schema: schemaOf(columns), layout }
    },
  }
}
