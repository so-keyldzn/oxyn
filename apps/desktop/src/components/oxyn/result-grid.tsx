import * as React from "react"
import { useQueries } from "@tanstack/react-query"
import { useVirtualizer } from "@tanstack/react-virtual"
import { HugeiconsIcon } from "@hugeicons/react"
import { Alert02Icon, Clock01Icon } from "@hugeicons/core-free-icons"

import { CellValue, cellText } from "@/components/oxyn/cell-value"
import {
  MAX_COPY_ROWS,
  copyText,
  inRange,
  rangeOf,
  rangeRows,
} from "@/components/oxyn/grid-selection"
import type { GridPosition } from "@/components/oxyn/grid-selection"
import { Alert, AlertDescription, AlertTitle } from "@/components/ui/alert"
import { Button } from "@/components/ui/button"
import {
  Empty,
  EmptyDescription,
  EmptyHeader,
  EmptyMedia,
  EmptyTitle,
} from "@/components/ui/empty"
import { Skeleton } from "@/components/ui/skeleton"
import type {
  Cell,
  ResultColumn,
  ResultPage,
  ResultWindow,
} from "@/lib/ipc/types"
import { cn } from "@/lib/utils"

/**
 * Compact density (docs/UX-SPEC.md, « Lisibilité et hauteur de grille »): the
 * height used until `--grid-row-height` is read from the page.
 */
export const ROW_HEIGHT = 24
export const HEADER_HEIGHT = 28
/** The largest page the grid asks for, in rows. */
export const PAGE_SIZE = 200
/** Cells one page aims at: 1 000 columns get 20 rows per call, not 200. */
const PAGE_CELLS = 20_000
const MIN_PAGE_ROWS = 20
/** Calls made to fill one page that the backend cut by its byte budget. */
const MAX_FILL_CALLS = 64
const GUTTER_WIDTH = 56
export const MIN_COLUMN_WIDTH = 48
export const MAX_AUTO_WIDTH = 480
export const MAX_COLUMN_WIDTH = 1200
/**
 * Advance of one Geist Mono character, as a share of the font size. Widths
 * follow the reading density (`--reading-text`): estimating at 12 px while the
 * grid draws at 14 px would cut every column short of its content.
 */
const CHAR_ADVANCE = 0.6
/** Advance at the compact preset's 12 px, used when the page is unreadable. */
const CHAR_WIDTH = 12 * CHAR_ADVANCE
const CELL_PADDING = 17

/**
 * What one page call answers — the wire type itself, not a copy of it.
 *
 * Both answers carry `type`: a page arrives as `{ type: "page", … }`. Telling
 * them apart by the *presence* of the tag read every page as expired, and the
 * grid drew « no longer available » over rows it was holding.
 */
export type PageAnswer = ResultWindow

export type FetchPage = (offset: number, limit: number) => Promise<PageAnswer>

/**
 * First element of every page query key. The backend formats the cells, and a
 * page is kept until invalidated: a change of format invalidates by this key.
 */
export const RESULT_PAGE_QUERY = "result-page"

function isExpired(answer: PageAnswer | undefined): boolean {
  return answer?.type === "expired"
}

function asPage(answer: PageAnswer | undefined): ResultPage | undefined {
  return answer?.type === "page" ? answer : undefined
}

/** Rows per page for a result this wide. Pure, so it is tested. */
export function pageSizeFor(columns: number) {
  return Math.max(
    MIN_PAGE_ROWS,
    Math.min(PAGE_SIZE, Math.floor(PAGE_CELLS / Math.max(1, columns)))
  )
}

/** Page indexes a visible row range needs. Pure, so it is tested without a DOM. */
export function pagesFor(first: number, last: number, pageSize = PAGE_SIZE) {
  if (last < first) return []
  const pages: Array<number> = []
  for (
    let page = Math.floor(first / pageSize);
    page <= Math.floor(last / pageSize);
    page++
  ) {
    pages.push(page)
  }
  return pages
}

/**
 * The Arrow indexes the grid draws, in order: every column the user has not
 * hidden. Hiding never renumbers a column — the inspector, the copy and the
 * export all speak of the same index. Pure, so it is tested.
 */
export function shownColumns(count: number, hidden?: ReadonlySet<number>) {
  const shown: Array<number> = []
  for (let column = 0; column < count; column++) {
    if (!hidden?.has(column)) shown.push(column)
  }
  return shown
}

/**
 * The shown column `step` places away from `column`, clamped to the ends. A
 * hidden `column` counts from the first shown column after it. Pure, so it is
 * tested.
 */
export function stepColumn(
  shown: ReadonlyArray<number>,
  column: number,
  step: number
) {
  let position = shown.findIndex((index) => index >= column)
  let moves = step
  if (position === -1) position = shown.length
  // Standing on a hidden column, the next shown one is already a step right.
  else if (shown[position] !== column && moves > 0) moves--
  const next = Math.max(0, Math.min(shown.length - 1, position + moves))
  return shown[next] ?? column
}

/** Integer, float and decimal columns read right-aligned. */
export function isNumericType(dataType: string) {
  return /^(U?Int\d+|Float\d+|Decimal(32|64|128|256)?\b)/.test(dataType)
}

/**
 * A column's first width, from its header and the rows already loaded.
 * Pure, so it is tested. The user's resize always wins over it.
 */
export function estimateWidth(
  column: ResultColumn,
  sample: ReadonlyArray<Cell | undefined>,
  charWidth = CHAR_WIDTH
) {
  let chars = Math.max(column.name.length, column.dataType.length * 0.85)
  for (const cell of sample) {
    if (cell === undefined) continue
    const length =
      cell === null
        ? 6
        : typeof cell === "object" && "text" in cell
          ? cell.text.length + 8
          : cellText(cell).length
    chars = Math.max(chars, Math.min(length, 80))
  }
  return Math.round(
    Math.max(
      MIN_COLUMN_WIDTH,
      Math.min(MAX_AUTO_WIDTH, chars * charWidth + CELL_PADDING)
    )
  )
}

/**
 * Asks `fetchPage` until `[offset, offset + limit)` is covered: the backend
 * may return fewer rows than asked when they weigh more than its byte budget
 * (`MAX_PAGE_BYTES`). Stops on an empty answer or after a bounded number of
 * calls, never loops on a result that stopped growing.
 */
export async function fillPage(
  fetchPage: FetchPage,
  offset: number,
  limit: number
): Promise<PageAnswer> {
  const first = await fetchPage(offset, limit)
  if (first.type !== "page") return first
  let rows = first.rows
  let last = first
  let calls = 1
  while (rows.length < limit && calls < MAX_FILL_CALLS) {
    const next = offset + rows.length
    if (next >= last.totalRows) break
    const answer = await fetchPage(next, limit - rows.length)
    calls++
    if (answer.type !== "page") return answer
    if (answer.rows.length === 0) break
    rows = rows.concat(answer.rows)
    last = answer
  }
  return { ...last, offset, rows }
}

interface ResultGridProps {
  resultKey: string
  columns: Array<ResultColumn>
  rowCount: number
  fetchPage: FetchPage
  className?: string
  /** Arrow indexes of the columns not drawn; the rows received keep them. */
  hiddenColumns?: ReadonlySet<number>
  /** Rows a search matched, marked in the gutter; never hidden or reordered. */
  matches?: ReadonlySet<number>
  /** Moves the active cell to a row; `key` changes to reveal the same row again. */
  reveal?: { row: number; key: number } | null
  /** The active cell, and its formatted value when its page is loaded. */
  onActiveChange?: (active: GridPosition | null, cell: Cell | undefined) => void
  /** Enter or a double click on a cell: open its full value. */
  onInspect?: (position: GridPosition) => void
  "aria-label"?: string
}

/**
 * What the reading density sets: the row height (`--grid-row-height`) and the
 * text size (`--reading-text`). Both are followed when the density changes —
 * the virtualizer must place rows where CSS draws them, or rows overlap after
 * a switch to Comfortable, and column widths must be estimated at the size the
 * cells are actually drawn at.
 */
function useGridMetrics(element: React.RefObject<HTMLElement | null>) {
  const [metrics, setMetrics] = React.useState({
    rowHeight: ROW_HEIGHT,
    charWidth: CHAR_WIDTH,
  })
  React.useEffect(() => {
    const read = () => {
      const target = element.current ?? document.documentElement
      const style = getComputedStyle(target)
      const height = Number.parseFloat(
        style.getPropertyValue("--grid-row-height")
      )
      const font = Number.parseFloat(style.getPropertyValue("--reading-text"))
      setMetrics((current) => {
        const next = {
          rowHeight:
            Number.isFinite(height) && height > 0 ? height : ROW_HEIGHT,
          charWidth:
            Number.isFinite(font) && font > 0
              ? font * CHAR_ADVANCE
              : CHAR_WIDTH,
        }
        return current.rowHeight === next.rowHeight &&
          current.charWidth === next.charWidth
          ? current
          : next
      })
    }
    read()
    const observer = new MutationObserver(read)
    observer.observe(document.documentElement, {
      attributes: true,
      attributeFilter: ["data-density", "class", "style"],
    })
    return () => observer.disconnect()
  }, [element])
  return metrics
}

/**
 * A virtualized, paged view over a result the backend holds.
 *
 * Only the pages around the viewport are ever in memory: scrolling to row
 * 40 000 000 asks for one page, it never loads what lies before it (I-06).
 * Columns are virtualized too, so a result of 1 000 columns draws the dozen
 * on screen. While the stream runs, `rowCount` grows and the last partial
 * page is asked again — its query key carries how many rows it can hold.
 *
 * Selecting and copying work on what is shown: a copy is bounded to one page
 * call and refused when a value in it is truncated. Nothing here filters or
 * sorts rows: `matches` only marks where a search found something.
 */
export const ResultGrid = React.memo(function ResultGrid({
  resultKey,
  columns,
  rowCount,
  fetchPage,
  className,
  hiddenColumns,
  matches,
  reveal,
  onActiveChange,
  onInspect,
  "aria-label": ariaLabel = "Result rows",
}: ResultGridProps) {
  const baseId = React.useId()
  const scrollRef = React.useRef<HTMLDivElement>(null)
  const [anchor, setAnchor] = React.useState<GridPosition | null>(null)
  const [active, setActive] = React.useState<GridPosition | null>(null)
  const [status, setStatus] = React.useState<{
    text: string
    failed: boolean
  } | null>(null)
  const range = rangeOf(anchor, active)
  // Pages are sized on the result's width, not on what is shown: hiding a
  // column must not ask the backend again for pages already held.
  const pageSize = pageSizeFor(columns.length)
  const shown = React.useMemo(
    () => shownColumns(columns.length, hiddenColumns),
    [columns.length, hiddenColumns]
  )
  const firstShown = shown[0] ?? 0
  const lastShown = shown[shown.length - 1] ?? 0
  // The active cell never stays on a column the user just hid.
  if (active && shown.length > 0 && !shown.includes(active.column)) {
    const next = {
      row: active.row,
      column: stepColumn(shown, active.column, 0),
    }
    setActive(next)
    setAnchor(next)
  }

  // Widths: the user's resize, else the estimate from the first page, else
  // the header alone. Reset when the result changes.
  const [resized, setResized] = React.useState<Record<number, number>>({})
  const [estimated, setEstimated] = React.useState<{
    charWidth: number
    widths: Array<number>
  } | null>(null)
  const [widthsFor, setWidthsFor] = React.useState(resultKey)
  if (widthsFor !== resultKey) {
    setWidthsFor(resultKey)
    setResized({})
    setEstimated(null)
    setAnchor(null)
    setActive(null)
    setStatus(null)
  }
  const { rowHeight, charWidth } = useGridMetrics(scrollRef)
  const autoWidth = (column: number) => {
    const estimate = estimated?.widths[column]
    if (estimate !== undefined) return estimate
    const described = columns[column]
    return described
      ? estimateWidth(described, [], charWidth)
      : MIN_COLUMN_WIDTH
  }
  const widthOf = (column: number) => resized[column] ?? autoWidth(column)

  const rows = useVirtualizer({
    count: rowCount,
    getScrollElement: () => scrollRef.current,
    estimateSize: () => rowHeight,
    overscan: 12,
  })
  React.useEffect(() => {
    rows.measure()
    // `rows` is a stable virtualizer instance; only the height re-measures.
  }, [rowHeight])
  const cols = useVirtualizer({
    horizontal: true,
    count: shown.length,
    getScrollElement: () => scrollRef.current,
    // The virtualizer counts drawn columns; widths belong to Arrow indexes.
    estimateSize: (position) => widthOf(shown[position] ?? position),
    overscan: 3,
    paddingStart: GUTTER_WIDTH,
  })
  React.useEffect(() => {
    cols.measure()
  }, [resized, estimated, shown, cols])

  const items = rows.getVirtualItems()
  const visibleColumns = cols.getVirtualItems()
  const first = items[0]?.index ?? 0
  const last = items[items.length - 1]?.index ?? -1
  const pages = pagesFor(first, last, pageSize)

  const pageQueries = useQueries({
    queries: pages.map((page) => {
      const offset = page * pageSize
      const holds = Math.min(pageSize, Math.max(0, rowCount - offset))
      return {
        queryKey: [
          RESULT_PAGE_QUERY,
          resultKey,
          pageSize,
          page,
          holds,
        ] as const,
        queryFn: () => fillPage(fetchPage, offset, holds),
        staleTime: Number.POSITIVE_INFINITY,
        // A result page is cheap to ask again and expensive to keep: pages
        // scrolled away from are released quickly.
        gcTime: 15_000,
        placeholderData: (previous: PageAnswer | undefined) => previous,
      }
    }),
  })

  const expired = pageQueries.some((query) => isExpired(query.data))
  const failedPage = pageQueries.find((query) => query.isError)

  const rowAt = (index: number): Array<Cell> | undefined => {
    const page = Math.floor(index / pageSize)
    const position = pages.indexOf(page)
    const data = asPage(pageQueries[position]?.data)
    if (!data || data.offset !== page * pageSize) return undefined
    return data.rows[index - data.offset]
  }

  // The first page that arrives gives each column its width; it is measured
  // again when the reading density changes the size the cells are drawn at.
  const firstRows = asPage(pageQueries[0]?.data)?.rows
  React.useEffect(() => {
    if (estimated?.charWidth === charWidth) return
    if (!firstRows || firstRows.length === 0) return
    setEstimated({
      charWidth,
      widths: columns.map((column, index) =>
        estimateWidth(
          column,
          firstRows.slice(0, 50).map((row) => row[index]),
          charWidth
        )
      ),
    })
  }, [firstRows, estimated, columns, charWidth])

  const activeCell = active ? rowAt(active.row)?.[active.column] : undefined
  React.useEffect(() => {
    onActiveChange?.(active, activeCell)
    // `onActiveChange` is a parent's callback; its identity must not re-fire it.
  }, [active?.row, active?.column, activeCell])

  React.useEffect(() => {
    if (!reveal || reveal.row >= rowCount) return
    const next = { row: reveal.row, column: active?.column ?? firstShown }
    setAnchor(next)
    setActive(next)
    rows.scrollToIndex(reveal.row, { align: "center" })
    // Revealing is driven by `key` alone: the active column is kept, not watched.
  }, [reveal?.key])

  const totalWidth = cols.getTotalSize()
  const cellId = (row: number, column: number) => `${baseId}-r${row}-c${column}`
  const activeRendered =
    active !== null &&
    items.some((item) => item.index === active.row) &&
    visibleColumns.some((column) => shown[column.index] === active.column)

  const copy = async (withHeaders: boolean) => {
    if (!range) return
    const count = rangeRows(range)
    if (count > MAX_COPY_ROWS) {
      const refused = copyText([], columns, range, withHeaders, hiddenColumns)
      if (!refused.ok) setStatus({ text: refused.reason, failed: true })
      return
    }
    try {
      // One bounded read for the selection, whatever is on screen: the rows
      // copied are the rows selected, not the ones that happen to be drawn.
      const page = asPage(await fillPage(fetchPage, range.top, count))
      if (!page) {
        setStatus({
          text: "Not copied: this result is no longer available.",
          failed: true,
        })
        return
      }
      const copied = copyText(
        page.rows,
        columns,
        range,
        withHeaders,
        hiddenColumns
      )
      if (!copied.ok) {
        setStatus({ text: copied.reason, failed: true })
        return
      }
      await navigator.clipboard.writeText(copied.text)
      setStatus({
        text: `Copied ${copied.cells.toLocaleString("en-US")} ${copied.cells === 1 ? "value" : "values"} from ${copied.rows.toLocaleString("en-US")} ${copied.rows === 1 ? "row" : "rows"}.`,
        failed: false,
      })
    } catch (error) {
      setStatus({
        text: `Not copied: ${error instanceof Error ? error.message : String(error)}`,
        failed: true,
      })
    }
  }

  const select = (next: GridPosition, extend: boolean) => {
    setActive(next)
    if (!extend) setAnchor(next)
    else if (!anchor) setAnchor(active ?? next)
    rows.scrollToIndex(next.row, { align: "auto" })
    const position = shown.indexOf(next.column)
    if (position !== -1) cols.scrollToIndex(position, { align: "auto" })
  }

  const onKeyDown = (event: React.KeyboardEvent<HTMLDivElement>) => {
    if (rowCount === 0 || shown.length === 0) return
    const mod = event.metaKey || event.ctrlKey
    if (mod && event.key.toLowerCase() === "c") {
      event.preventDefault()
      void copy(event.shiftKey)
      return
    }
    if (event.key === "Enter" && active) {
      event.preventDefault()
      onInspect?.(active)
      return
    }
    if (event.key === "Escape" && range && rangeRows(range) > 1) {
      event.preventDefault()
      setAnchor(active)
      return
    }
    const current = active ?? { row: 0, column: firstShown }
    const lastRow = rowCount - 1
    let next = current
    switch (event.key) {
      case "ArrowDown":
        next = { ...current, row: Math.min(lastRow, current.row + 1) }
        break
      case "ArrowUp":
        next = { ...current, row: Math.max(0, current.row - 1) }
        break
      case "ArrowRight":
        next = { ...current, column: stepColumn(shown, current.column, 1) }
        break
      case "ArrowLeft":
        next = { ...current, column: stepColumn(shown, current.column, -1) }
        break
      case "PageDown":
        next = { ...current, row: Math.min(lastRow, current.row + 20) }
        break
      case "PageUp":
        next = { ...current, row: Math.max(0, current.row - 20) }
        break
      case "Home":
        next = mod
          ? { row: 0, column: firstShown }
          : { ...current, column: firstShown }
        break
      case "End":
        next = mod
          ? { row: lastRow, column: lastShown }
          : { ...current, column: lastShown }
        break
      default:
        return
    }
    event.preventDefault()
    select(next, event.shiftKey)
  }

  const resize = (column: number, width: number | null) =>
    setResized((current) => {
      const next = { ...current }
      if (width === null) delete next[column]
      else
        next[column] = Math.round(
          Math.max(MIN_COLUMN_WIDTH, Math.min(MAX_COLUMN_WIDTH, width))
        )
      return next
    })

  if (expired) {
    return (
      <Empty className={cn("h-full border-0", className)}>
        <EmptyHeader>
          <EmptyMedia variant="icon">
            <HugeiconsIcon icon={Clock01Icon} strokeWidth={2} />
          </EmptyMedia>
          <EmptyTitle>This result is no longer available</EmptyTitle>
          <EmptyDescription>
            Retention released it. Oxyn never runs the query again to bring it
            back; run it yourself to see these rows.
          </EmptyDescription>
        </EmptyHeader>
      </Empty>
    )
  }

  // One resize handle takes Tab: the active column's, so the header does not
  // add a stop per column.
  const tabbableHandle = active?.column ?? firstShown

  return (
    <div className={cn("flex h-full min-h-0 flex-col", className)}>
      <div
        ref={scrollRef}
        role="grid"
        aria-label={ariaLabel}
        aria-rowcount={rowCount + 1}
        aria-colcount={shown.length + 1}
        aria-multiselectable
        aria-activedescendant={
          active && activeRendered
            ? cellId(active.row, active.column)
            : undefined
        }
        tabIndex={0}
        onKeyDown={onKeyDown}
        className="relative min-h-0 flex-1 overflow-auto bg-background font-mono text-[length:var(--reading-text)] outline-none focus-visible:ring-2 focus-visible:ring-ring focus-visible:ring-inset"
      >
        <div
          // The sizer sits between the grid and its rows: without a role, it
          // breaks the `grid` → `row` ownership screen readers walk.
          role="rowgroup"
          className="relative"
          style={{
            width: totalWidth,
            height: rows.getTotalSize() + HEADER_HEIGHT,
          }}
        >
          <div
            role="row"
            aria-rowindex={1}
            className="sticky top-0 z-10 border-b border-grid-line bg-grid-header font-sans"
            style={{ width: totalWidth, height: HEADER_HEIGHT }}
          >
            <div
              role="columnheader"
              aria-colindex={1}
              className="sticky left-0 z-10 flex h-full items-center justify-end border-r border-grid-line bg-grid-header px-2 text-[length:var(--reading-caption)] text-muted-foreground"
              style={{ width: GUTTER_WIDTH }}
            >
              <span className="sr-only">Row number</span>
            </div>
            {visibleColumns.map((virtual) => {
              const index = shown[virtual.index] ?? virtual.index
              const column = columns[index]
              if (!column) return null
              const numeric = isNumericType(column.dataType)
              return (
                <div
                  key={virtual.key}
                  role="columnheader"
                  aria-colindex={virtual.index + 2}
                  title={`${column.name} · ${column.dataType}`}
                  className={cn(
                    "absolute top-0 flex h-full min-w-0 flex-col justify-center border-r border-grid-line px-2 leading-3",
                    numeric && "items-end text-right"
                  )}
                  style={{ left: virtual.start, width: virtual.size }}
                >
                  <span className="max-w-full truncate text-[length:var(--reading-text)] font-medium text-foreground">
                    {column.name}
                  </span>
                  <span className="max-w-full truncate font-mono text-[length:var(--reading-caption)] text-muted-foreground">
                    {column.dataType}
                  </span>
                  <ResizeHandle
                    name={column.name}
                    width={widthOf(index)}
                    tabbable={index === tabbableHandle}
                    onResize={(width) => resize(index, width)}
                    onReset={() => resize(index, null)}
                  />
                </div>
              )
            })}
          </div>

          {items.map((item) => {
            const row = rowAt(item.index)
            const matched = matches?.has(item.index) ?? false
            const isActiveRow = active?.row === item.index
            const striped = item.index % 2 === 1
            return (
              <div
                key={item.key}
                role="row"
                aria-rowindex={item.index + 2}
                data-match={matched || undefined}
                className={cn(
                  // `top-0` matters: without it an absolute row starts at its
                  // static position — under the sticky header, already 28 px
                  // down — and the translate below adds the header again. The
                  // first row then sits a row's height under the header, and
                  // the last one 28 px below the end of the scroll range,
                  // where no scrolling can reach it.
                  "absolute top-0 left-0 border-b border-grid-line/50",
                  striped && "bg-grid-stripe",
                  isActiveRow && "bg-accent"
                )}
                style={{
                  width: totalWidth,
                  height: rowHeight,
                  transform: `translateY(${item.start + HEADER_HEIGHT}px)`,
                }}
              >
                <div
                  role="rowheader"
                  aria-colindex={1}
                  aria-label={`Row ${item.index + 1}${matched ? ", matches the search" : ""}`}
                  onMouseDown={(event) => {
                    if (shown.length === 0) return
                    event.preventDefault()
                    scrollRef.current?.focus()
                    const start =
                      event.shiftKey && anchor ? anchor.row : item.index
                    setAnchor({ row: start, column: firstShown })
                    setActive({ row: item.index, column: lastShown })
                  }}
                  className={cn(
                    // Opaque, so cells scrolled under it stay hidden; the
                    // stripe and the active row are drawn over it again.
                    "sticky left-0 z-[1] flex h-full cursor-pointer items-center justify-end border-r border-grid-line bg-background px-2 text-[length:var(--reading-caption)] text-muted-foreground tabular-nums",
                    striped &&
                      "bg-[linear-gradient(var(--color-grid-stripe),var(--color-grid-stripe))]",
                    isActiveRow && "bg-accent text-foreground",
                    matched && "shadow-[inset_2px_0_0_var(--color-primary)]"
                  )}
                  style={{ width: GUTTER_WIDTH }}
                >
                  {item.index + 1}
                </div>
                {visibleColumns.map((virtual) => {
                  const column = shown[virtual.index] ?? virtual.index
                  const described = columns[column]
                  const isActive = isActiveRow && active.column === column
                  const selected = inRange(range, item.index, column)
                  const cell = row?.[column]
                  const numeric = described
                    ? isNumericType(described.dataType)
                    : false
                  const text = typeof cell === "string" ? cell : null
                  return (
                    <div
                      key={virtual.key}
                      id={cellId(item.index, column)}
                      role="gridcell"
                      aria-colindex={virtual.index + 2}
                      aria-selected={selected}
                      onMouseDown={(event) =>
                        select({ row: item.index, column }, event.shiftKey)
                      }
                      onDoubleClick={() =>
                        onInspect?.({ row: item.index, column })
                      }
                      className={cn(
                        "absolute top-0 flex h-full items-center overflow-hidden border-r border-grid-line/50 px-2",
                        numeric && "justify-end text-right tabular-nums",
                        selected && !isActive && "bg-primary/10",
                        isActive &&
                          "outline-2 -outline-offset-2 outline-primary"
                      )}
                      style={{ left: virtual.start, width: virtual.size }}
                    >
                      {row ? (
                        <CellValue
                          cell={cell ?? null}
                          title={
                            text !== null &&
                            text.length * charWidth >
                              virtual.size - CELL_PADDING
                              ? text.slice(0, 2000)
                              : undefined
                          }
                        />
                      ) : (
                        <Skeleton className="h-2.5 w-3/4" />
                      )}
                    </div>
                  )
                })}
              </div>
            )
          })}
        </div>
      </div>

      {failedPage ? (
        <Alert
          variant="destructive"
          className="rounded-none border-x-0 border-b-0 py-2"
        >
          <HugeiconsIcon icon={Alert02Icon} strokeWidth={2} />
          <AlertTitle>This page could not be read</AlertTitle>
          <AlertDescription className="flex flex-wrap items-center gap-2 text-foreground">
            <span data-selectable className="font-mono">
              {failedPage.error instanceof Error
                ? failedPage.error.message
                : String(failedPage.error)}
            </span>
            <span className="text-muted-foreground">
              The query was not run again.
            </span>
            <Button
              size="xs"
              variant="outline"
              onClick={() => void failedPage.refetch()}
            >
              Retry local read
            </Button>
          </AlertDescription>
        </Alert>
      ) : null}

      <p
        role="status"
        className={cn(
          "min-h-0 shrink-0 px-2 font-sans text-[length:var(--reading-caption)] empty:hidden",
          status?.failed ? "text-warning" : "text-muted-foreground"
        )}
      >
        {status?.text}
      </p>
    </div>
  )
})

/**
 * A column's resize handle: a focusable separator, driven by the pointer or
 * by ←/→ (Shift for larger steps) and Home for the automatic width.
 */
function ResizeHandle({
  name,
  width,
  tabbable,
  onResize,
  onReset,
}: {
  name: string
  width: number
  tabbable: boolean
  onResize: (width: number) => void
  onReset: () => void
}) {
  const drag = React.useRef<{ x: number; width: number } | null>(null)
  return (
    <div
      role="separator"
      aria-orientation="vertical"
      aria-label={`Resize column ${name}`}
      aria-valuenow={width}
      aria-valuemin={MIN_COLUMN_WIDTH}
      aria-valuemax={MAX_COLUMN_WIDTH}
      tabIndex={tabbable ? 0 : -1}
      onKeyDown={(event) => {
        const step = event.shiftKey ? 64 : 16
        if (event.key === "ArrowLeft") onResize(width - step)
        else if (event.key === "ArrowRight") onResize(width + step)
        else if (event.key === "Home") onReset()
        else return
        // The grid moves the active cell on the same keys.
        event.preventDefault()
        event.stopPropagation()
      }}
      onPointerDown={(event) => {
        event.preventDefault()
        event.stopPropagation()
        event.currentTarget.setPointerCapture(event.pointerId)
        drag.current = { x: event.clientX, width }
      }}
      onPointerMove={(event) => {
        if (!drag.current) return
        onResize(drag.current.width + event.clientX - drag.current.x)
      }}
      onPointerUp={() => {
        drag.current = null
      }}
      onMouseDown={(event) => event.stopPropagation()}
      onDoubleClick={onReset}
      className="absolute top-0 -right-1 z-[2] h-full w-2 cursor-col-resize touch-none outline-none select-none after:absolute after:inset-y-1 after:left-1/2 after:w-px after:-translate-x-1/2 hover:after:bg-primary/60 focus-visible:after:w-0.5 focus-visible:after:bg-ring motion-safe:after:transition-colors"
    />
  )
}
