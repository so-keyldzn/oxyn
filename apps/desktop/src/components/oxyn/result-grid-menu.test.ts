import { describe, expect, it, vi } from "vitest"

import {
  gridMenuSource,
  loadedRowsLabel,
  selectionCopy,
  valueCopy,
} from "./result-grid-menu"
import type { GridMenu, GridMenuHost } from "./result-grid-menu"
import type { ResultColumn } from "@/lib/ipc/types"

const columns: Array<ResultColumn> = [
  { name: "id", dataType: "Int64", nullable: false },
  { name: "email", dataType: "Utf8", nullable: true },
  { name: "note", dataType: "Utf8", nullable: true },
]

function hostOf(
  menu: Partial<GridMenu>,
  target: GridMenuHost["target"],
  overrides: Partial<GridMenuHost> = {}
): GridMenuHost {
  return {
    menu: {
      origin: "query",
      relation: false,
      ai: { kind: "none" },
      copyText: vi.fn(),
      copyRows: vi.fn(),
      filterable: false,
      sortable: false,
      ...menu,
    },
    target,
    columns,
    shown: [0, 1, 2],
    rowCount: 120,
    range: { top: 4, bottom: 9, left: 0, right: 2 },
    readCell: () => Promise.resolve("x"),
    autosize: vi.fn(),
    report: vi.fn(),
    ...overrides,
  }
}

const anchor = {} as Element

describe("selectionCopy", () => {
  it("copies the selected rows and the shown columns in the range", () => {
    expect(
      selectionCopy({ top: 3, bottom: 7, left: 1, right: 4 }, [0, 1, 3, 4, 6])
    ).toEqual({ offset: 3, count: 5, columns: [1, 3, 4] })
  })
})

describe("valueCopy", () => {
  it("copies a value as the grid shows it", () => {
    expect(valueCopy("a\tb")).toMatchObject({ ok: true, text: '"a\tb"' })
    expect(valueCopy(null)).toMatchObject({ ok: true, text: "" })
  })

  it("refuses a truncated value rather than shortening it", () => {
    expect(valueCopy({ text: "Long…", fullBytes: 9000 })).toMatchObject({
      ok: false,
    })
  })
})

describe("loadedRowsLabel", () => {
  it("names loaded rows, never the column", () => {
    expect(loadedRowsLabel(1)).toBe("1 loaded row")
    expect(loadedRowsLabel(1200)).toBe("1,200 loaded rows")
  })
})

describe("gridMenuSource", () => {
  it("offers no order or filter handler on a query's rows", () => {
    const { state, actions } = gridMenuSource(
      hostOf({}, { kind: "cell", position: { row: 5, column: 1 }, anchor })
    )
    expect(state).toMatchObject({
      target: "cell",
      origin: "query",
      selectedRows: 6,
      oneColumn: false,
      loadedRows: 120,
    })
    expect(state).toMatchObject({ sortable: false, filterable: false })
    expect(actions.sort).toBeUndefined()
    expect(actions.filter).toBeUndefined()
    expect(actions.copyValue).toBeDefined()
    expect(actions.autosize).toBeUndefined()
  })

  it("sorts a preview by the column's name", () => {
    const sort = vi.fn()
    const { state, actions } = gridMenuSource(
      hostOf(
        { origin: "preview", sortable: true, sort },
        { kind: "header", column: 2, anchor }
      )
    )
    expect(state).toMatchObject({ sortable: true, filterable: false })
    expect(actions.filter).toBeUndefined()
    actions.sort?.(true)
    expect(sort).toHaveBeenCalledWith("note", true)
  })

  it("copies the loaded rows of one column, never more", () => {
    const copyRows = vi.fn()
    const { actions } = gridMenuSource(
      hostOf({ copyRows }, { kind: "header", column: 1, anchor })
    )
    actions.copyValues?.()
    expect(copyRows).toHaveBeenCalledWith({
      offset: 0,
      count: 120,
      columns: [1],
      format: "tsv",
      header: false,
      what: "Values of email",
    })
  })

  it("never hides the last shown column", () => {
    const { actions } = gridMenuSource(
      hostOf(
        { hideColumn: vi.fn() },
        { kind: "header", column: 1, anchor },
        { shown: [1] }
      )
    )
    expect(actions.hideColumn).toBeUndefined()
  })
})
