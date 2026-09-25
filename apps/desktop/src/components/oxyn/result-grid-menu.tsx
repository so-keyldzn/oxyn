// The context menus of the result grid (docs/UX-SPEC.md, « Menus
// contextuels », Grille and En-tête de colonne): a cell or the selection
// around it, and a column header. Every entry is a registry action
// (docs/adr/0041-registre-d-actions-menus-et-raccourcis.md, point 6); this
// file only says what was right-clicked and which handlers the grid's
// source offers.

import * as React from "react"

import { ActionMenuContent } from "@/components/oxyn/action-menu-items"
import { copyText, rangeRows } from "@/components/oxyn/grid-selection"
import type { GridPosition, GridRange } from "@/components/oxyn/grid-selection"
import { ContextMenu, ContextMenuTrigger } from "@/components/ui/context-menu"
import type {
  CopyRowsFormat,
  GridAiLevel,
  GridMenuActions,
  GridMenuState,
  GridOrigin,
} from "@/lib/actions/targets"
import type { Cell, ResultColumn } from "@/lib/ipc/types"

/** Rows of the result to copy, read and formatted in Rust (never here). */
export interface RowsCopy {
  offset: number
  count: number
  /** Arrow indexes, in the order the grid shows them. */
  columns: Array<number>
  format: CopyRowsFormat
  /** TSV and CSV: a header line. */
  header: boolean
  /** What the confirmation names: « 3 rows as CSV ». */
  what: string
}

/**
 * What the menu needs from where the rows come from — a console, a retained
 * result, a table preview. `src/features` builds it; a story fakes it.
 */
export interface GridMenuSource {
  origin: GridOrigin
  /** The rows come from one known relation: `INSERT` can name it. */
  relation: boolean
  ai: GridAiLevel
  /**
   * Copies text the user asked for, and says whether it worked. Called in
   * the click; a text still to be read is a promise (`writeClipboard`).
   */
  copyText: (text: string | Promise<string>, what: string) => void
  copyRows: (request: RowsCopy) => void
  /** The preview declares a filter (`PREVIEW_FILTER`); false for a query. */
  filterable: boolean
  /** The preview declares an order (`PREVIEW_SORT`); false for a query. */
  sortable: boolean
  /** The preview's order, by column name; absent where none is declared. */
  sort?: (column: string, descending: boolean) => void
  /** Brings the preview's filter to this column; absent where none is declared. */
  filter?: (column: string) => void
}

/** The source, and what the panel around the grid adds: hiding a column. */
export interface GridMenu extends GridMenuSource {
  hideColumn?: (column: number) => void
}

/** What a right click landed on, and the element it landed on. */
export type GridMenuTarget =
  | { kind: "cell"; position: GridPosition; anchor: Element }
  | { kind: "header"; column: number; anchor: Element }

const FORMAT_LABEL: Record<CopyRowsFormat, string> = {
  tsv: "TSV",
  csv: "CSV",
  json: "JSON",
  markdown: "Markdown",
  insert: "INSERT",
  inList: "IN list",
}

function indexIn(element: HTMLElement, key: "row" | "column") {
  const value = Number(element.dataset[key])
  return Number.isInteger(value) && value >= 0 ? value : null
}

/**
 * What `element`, inside the grid, stands for. The menu key and ⇧F10 fire on
 * the grid itself: they act on the active cell, as a right click acts on the
 * cell under the pointer.
 */
export function menuTargetOf(
  element: Element,
  active: GridPosition | null
): GridMenuTarget | null {
  const cell = element.closest<HTMLElement>('[role="gridcell"]')
  if (cell) {
    const row = indexIn(cell, "row")
    const column = indexIn(cell, "column")
    if (row !== null && column !== null)
      return { kind: "cell", position: { row, column }, anchor: cell }
  }
  const header = element.closest<HTMLElement>('[role="columnheader"]')
  if (header) {
    const column = indexIn(header, "column")
    if (column !== null) return { kind: "header", column, anchor: header }
  }
  return active ? { kind: "cell", position: active, anchor: element } : null
}

/**
 * The rows and shown columns a copy of `range` carries. The range spans
 * drawn positions, so a hidden column is left out and a moved one is taken
 * where it stands, as `⌘C` takes them. Pure, so it is tested.
 */
export function selectionCopy(range: GridRange, shown: ReadonlyArray<number>) {
  return {
    offset: range.top,
    count: rangeRows(range),
    columns: shown.slice(range.left, range.right + 1),
  }
}

/**
 * One value, as the grid shows it: refused when truncated or unrenderable,
 * like any copy from the grid (`copyText`). Pure, so it is tested.
 */
export function valueCopy(cell: Cell) {
  return copyText(
    [[cell]],
    [],
    { top: 0, bottom: 0, left: 0, right: 0 },
    false,
    [0]
  )
}

/** « 1 loaded row », « 1,200 loaded rows ». Pure, so it is tested. */
export function loadedRowsLabel(rows: number) {
  return `${rows.toLocaleString("en-US")} loaded ${rows === 1 ? "row" : "rows"}`
}

/** What the grid hands its menu when it opens. */
export interface GridMenuHost {
  menu: GridMenu
  target: GridMenuTarget
  columns: ReadonlyArray<ResultColumn>
  /** Arrow indexes drawn, in order. */
  shown: ReadonlyArray<number>
  /** Rows the result holds now. */
  rowCount: number
  /** The selection, in drawn positions: columns index `shown`. */
  range: GridRange | null
  /** The cell's value if its page is loaded; `undefined` otherwise. */
  loadedCell: (position: GridPosition) => Cell | undefined
  /** The cell's value, read from a loaded page or with one bounded read. */
  readCell: (position: GridPosition) => Promise<Cell | undefined>
  onInspect?: (position: GridPosition) => void
  /** Back to the automatic width (`resize(column, null)`). */
  autosize: (column: number) => void
  /** Says a refused copy in the grid's status line. */
  report: (text: string) => void
}

/**
 * The registry's view of the target, and the handlers that act on it.
 * `afterClose` runs what moves the keyboard elsewhere once the menu is gone:
 * closing gives the focus back to the grid, over wherever it was sent.
 */
export function gridMenuSource(
  host: GridMenuHost,
  afterClose: (run: () => void) => void = (run) => run()
): {
  state: GridMenuState
  actions: GridMenuActions
} {
  const { menu, target, columns, shown, rowCount, range } = host
  const column = target.kind === "cell" ? target.position.column : target.column
  const name = columns[column]?.name ?? ""
  const selection =
    target.kind === "cell" && range ? selectionCopy(range, shown) : null
  const { sort, filter } = menu
  const hide = menu.hideColumn
  const state: GridMenuState = {
    target: target.kind,
    origin: menu.origin,
    filterable: menu.filterable,
    sortable: menu.sortable,
    selectedRows: selection?.count ?? 0,
    oneColumn: selection ? selection.columns.length === 1 : true,
    relation: menu.relation,
    loadedRows: rowCount,
    shownColumns: shown.length,
    ai: menu.ai,
  }
  const actions: GridMenuActions = {
    sort: sort && ((descending) => sort(name, descending)),
    filter: filter && (() => afterClose(() => filter(name))),
    hideColumn: hide && (() => hide(column)),
  }
  if (target.kind === "cell") {
    const position = target.position
    actions.copyValue = () => {
      const loaded = host.loadedCell(position)
      if (loaded !== undefined) {
        const copied = valueCopy(loaded)
        if (copied.ok) menu.copyText(copied.text, "Value")
        else host.report(copied.reason)
        return
      }
      // Read again: the clipboard is asked in the click and given the value
      // afterwards (`writeClipboard`), and a failure is the copy's to say.
      menu.copyText(
        host.readCell(position).then((cell) => {
          if (cell === undefined)
            throw new Error("This value is no longer available.")
          const copied = valueCopy(cell)
          if (!copied.ok) throw new Error(copied.reason)
          return copied.text
        }),
        "Value"
      )
    }
    if (selection)
      actions.copyRows = (format) =>
        menu.copyRows({
          ...selection,
          format,
          header: true,
          what: `${selection.count.toLocaleString("en-US")} ${selection.count === 1 ? "row" : "rows"} as ${FORMAT_LABEL[format]}`,
        })
    const inspect = host.onInspect
    if (inspect) actions.inspect = () => inspect(position)
  } else {
    actions.autosize = () => host.autosize(column)
    actions.copyName = () => menu.copyText(name, "Column name")
    // The rows the result holds, never the column: the rest is not in
    // memory, and is not read for a copy (I-06).
    actions.copyValues = () =>
      menu.copyRows({
        offset: 0,
        count: rowCount,
        columns: [column],
        format: "tsv",
        header: false,
        what: `Values of ${name}`,
      })
  }
  return { state, actions }
}

const MenuEnabled = React.createContext(false)

/**
 * The grid's frame: `enabled`, a context menu on the target `host` describes;
 * otherwise — an agent's rows in the assistant — a plain frame.
 */
export function GridMenuRoot({
  enabled,
  host,
  children,
  ...frame
}: React.ComponentProps<"div"> & {
  enabled: boolean
  host: GridMenuHost | null
}) {
  const pending = React.useRef<(() => void) | null>(null)
  if (!enabled) return <div {...frame}>{children}</div>
  return (
    <MenuEnabled.Provider value>
      <ContextMenu
        onOpenChangeComplete={(open) => {
          if (open) return
          const run = pending.current
          pending.current = null
          run?.()
        }}
      >
        <div {...frame}>{children}</div>
        {host ? (
          <GridMenuContent
            host={host}
            afterClose={(run) => {
              pending.current = run
            }}
          />
        ) : null}
      </ContextMenu>
    </MenuEnabled.Provider>
  )
}

/** The scrolling grid itself: what a right click opens the menu on. */
export function GridMenuTrigger({
  children,
  ...grid
}: React.ComponentProps<"div">) {
  const enabled = React.useContext(MenuEnabled)
  if (!enabled) return <div {...grid}>{children}</div>
  return (
    <ContextMenuTrigger render={<div {...grid} />}>
      {children}
    </ContextMenuTrigger>
  )
}

function GridMenuContent({
  host,
  afterClose,
}: {
  host: GridMenuHost
  afterClose: (run: () => void) => void
}) {
  const source = gridMenuSource(host, afterClose)
  const target = host.target
  if (target.kind === "cell")
    return (
      <ActionMenuContent
        surface="gridCell"
        anchor={target.anchor}
        sources={{ grid: source }}
      />
    )
  return (
    <ActionMenuContent
      surface="gridHeader"
      anchor={target.anchor}
      sources={{ grid: source }}
      title={host.columns[target.column]?.name}
      details={{ "column.copyValues": loadedRowsLabel(host.rowCount) }}
    />
  )
}
