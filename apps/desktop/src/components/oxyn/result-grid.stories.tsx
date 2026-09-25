import type { Meta, StoryObj } from "@storybook/react-vite"
import { expect, fn, userEvent, waitFor } from "storybook/test"

import { invoiceColumns, syntheticPages } from "./fixtures"
import { centerOf, drag } from "./pointer-drag-fixtures"
import { ResultGrid } from "./result-grid"
import type { FetchPage } from "./result-grid"
import { modKey } from "@/lib/actions/platform"
import type { Cell, ResultColumn } from "@/lib/ipc/types"

/** A result served from a cell function, for the wide and odd cases. */
function pagesOf(
  rows: number,
  cell: (row: number, column: number) => Cell,
  columns: number,
  delayMs = 0
): FetchPage {
  return (offset, limit) =>
    new Promise((resolve) => {
      setTimeout(() => {
        const end = Math.min(rows, offset + limit)
        const page: Array<Array<Cell>> = []
        for (let row = offset; row < end; row++) {
          page.push(Array.from({ length: columns }, (_, c) => cell(row, c)))
        }
        resolve({
          type: "page",
          offset,
          rows: page,
          totalRows: rows,
          complete: true,
        })
      }, delayMs)
    })
}

const thousandColumns: Array<ResultColumn> = Array.from(
  { length: 1000 },
  (_, index) => ({
    name: `metric_${String(index).padStart(4, "0")}`,
    dataType: index % 3 === 0 ? "Float64" : "Utf8",
    nullable: true,
  })
)

const HOSTILE = 'users"; DROP TABLE audit; --'

const oddColumns: Array<ResultColumn> = [
  { name: HOSTILE, dataType: "Utf8", nullable: false },
  { name: "اسم_العميل", dataType: "Utf8", nullable: true },
  { name: "שם_לקוח", dataType: "Utf8", nullable: true },
  {
    name: "a_column_name_long_enough_to_never_fit_in_any_reasonable_header_width",
    dataType: "LargeUtf8",
    nullable: true,
  },
  { name: "payload", dataType: "LargeBinary", nullable: true },
  { name: "document", dataType: "Utf8", nullable: true },
  { name: "amount", dataType: "Decimal128(38, 10)", nullable: true },
]

function oddCell(row: number, column: number): Cell {
  switch (column) {
    case 0:
      return `<img src=x onerror=alert(${row})>`
    case 1:
      return row % 2 === 0 ? "شركة الأفق للتجارة" : null
    case 2:
      return row % 3 === 0 ? "NULL" : "חברת האופק בע״מ"
    case 3:
      return "🧪 naïve café — ｆｕｌｌｗｉｄｔｈ — 𝔘𝔫𝔦𝔠𝔬𝔡𝔢 ".repeat(4)
    case 4:
      return {
        text: "\\x89504e470d0a1a0a0000000d49484452",
        fullBytes: 52_428_800,
      }
    case 5:
      return {
        text: '{"customer":{"id":42,"tags":["a","b"],"history":[{"at":"2026-01-01"',
        fullBytes: 3_145_728,
      }
    default:
      return row % 5 === 0 ? null : "12345678901234567890.1234567890"
  }
}

// Typed as the grid takes it, so that stories may answer « expired ».
const oneMillionRows: FetchPage = syntheticPages(1_000_000)

const meta = {
  title: "Oxyn/ResultGrid",
  component: ResultGrid,
  decorators: [
    (Story) => (
      <div className="h-[560px]">
        <Story />
      </div>
    ),
  ],
  args: {
    resultKey: "story-1m",
    columns: invoiceColumns,
    rowCount: 1_000_000,
    fetchPage: oneMillionRows,
  },
} satisfies Meta<typeof ResultGrid>

export default meta
type Story = StoryObj<typeof meta>

export const OneMillionRows: Story = {
  play: async ({ canvas, args }) => {
    const grid = canvas.getByRole("grid", { name: "Result rows" })
    await expect(grid).toHaveAttribute(
      "aria-rowcount",
      String(args.rowCount + 1)
    )
    // Rows arrive page by page; the first page renders its first id.
    await waitFor(() =>
      expect(canvas.getAllByText("1").length).toBeGreaterThan(0)
    )
    // NULL is drawn as such, not as an empty cell.
    await waitFor(() =>
      expect(canvas.getAllByText("NULL").length).toBeGreaterThan(0)
    )

    // Keyboard reach: the grid takes focus and moves the active cell.
    grid.focus()
    await userEvent.keyboard("{ArrowDown}{ArrowRight}")
    await waitFor(() =>
      expect(canvas.getAllByRole("gridcell", { selected: true })).toHaveLength(
        1
      )
    )
  },
}

export const JumpToTheEnd: Story = {
  args: { resultKey: "story-jump" },
  play: async ({ canvas }) => {
    const grid = canvas.getByRole("grid")
    grid.focus()
    await userEvent.keyboard(`{${modKey}>}{End}{/${modKey}}`)
    // The last page is fetched directly, without loading what lies before.
    // By its row header: the text alone also matches that row's `id` cell,
    // which holds the same number, and the query then fails as ambiguous.
    await waitFor(
      () =>
        expect(
          canvas.getByRole("rowheader", { name: "Row 1000000" })
        ).toBeVisible(),
      { timeout: 3000 }
    )
  },
}

/**
 * Where rows are drawn, measured rather than « visible »: `toBeVisible` passes
 * on a row clipped out of the viewport, which is how a grid that lost its
 * first row's height under the header — and its last row below the scroll
 * range — passed every other story.
 */
export const FirstAndLastRowsAreInReach: Story = {
  args: { resultKey: "story-geometry", rowCount: 5_000 },
  play: async ({ canvas }) => {
    const grid = canvas.getByRole("grid")
    const rowAt = (index: number) =>
      grid.querySelector<HTMLElement>(`[role="row"][aria-rowindex="${index}"]`)

    // Row 1 is the header; the first data row starts where it ends.
    await waitFor(() => expect(rowAt(2)).not.toBeNull())
    const header = rowAt(1)!.getBoundingClientRect()
    await expect(
      Math.abs(rowAt(2)!.getBoundingClientRect().top - header.bottom)
    ).toBeLessThanOrEqual(1)

    grid.focus()
    await userEvent.keyboard(`{${modKey}>}{End}{/${modKey}}`)
    const lastIndex = 5_000 + 1
    await waitFor(() => expect(rowAt(lastIndex)).not.toBeNull(), {
      timeout: 3000,
    })
    await waitFor(() => {
      const bottom = grid.getBoundingClientRect().top + grid.clientHeight
      expect(
        rowAt(lastIndex)!.getBoundingClientRect().bottom
      ).toBeLessThanOrEqual(bottom + 1)
    })
  },
}

/**
 * A page answers tagged — `{ type: "page", … }` — exactly as the backend
 * answers it. The rows are drawn, and « no longer available » is nowhere:
 * telling a page from an expired answer by the *presence* of the tag made the
 * grid claim every result it held had been released.
 */
export const FewRows: Story = {
  args: {
    resultKey: "story-few",
    rowCount: 12,
    fetchPage: syntheticPages(12, 0),
  },
  play: async ({ canvas }) => {
    await waitFor(() =>
      expect(canvas.getAllByRole("gridcell").length).toBeGreaterThan(0)
    )
    expect(canvas.queryByText("This result is no longer available")).toBeNull()
  },
}

/** Retention released the result: said, and nothing runs the query again. */
export const Expired: Story = {
  args: {
    resultKey: "story-expired",
    rowCount: 40,
    fetchPage: () => Promise.resolve({ type: "expired" as const }),
  },
  play: async ({ canvas }) => {
    await expect(
      await canvas.findByText("This result is no longer available")
    ).toBeVisible()
    await expect(canvas.queryByRole("grid")).toBeNull()
  },
}

/**
 * A truncated value is not copied as if it were whole: the copy is refused
 * and says why, rather than putting a prefix on the clipboard.
 */
export const CopyRefusesATruncatedValue: Story = {
  args: {
    resultKey: "story-copy",
    rowCount: 12,
    fetchPage: syntheticPages(12, 0),
  },
  play: async ({ canvas }) => {
    const gutter = await canvas.findByRole("rowheader", { name: "Row 1" })
    await waitFor(() =>
      expect(canvas.getAllByText("Acme SA").length).toBeGreaterThan(0)
    )
    await userEvent.pointer({ keys: "[MouseLeft]", target: gutter })
    await expect(
      canvas.getAllByRole("gridcell", { selected: true })
    ).toHaveLength(7)
    await userEvent.keyboard(`{${modKey}>}c{/${modKey}}`)
    await waitFor(() =>
      expect(canvas.getByRole("status")).toHaveTextContent(
        /Not copied: 1 selected value is truncated/
      )
    )
  },
}

const onInspect = fn()

export const SearchMatches: Story = {
  args: {
    resultKey: "story-matches",
    rowCount: 12,
    fetchPage: syntheticPages(12, 0),
    matches: new Set([1, 7]),
    reveal: { row: 7, key: 1 },
    onInspect,
  },
  play: async ({ canvas }) => {
    await expect(
      await canvas.findByRole("rowheader", {
        name: "Row 8, matches the search",
      })
    ).toBeVisible()
    await expect(canvas.getByRole("rowheader", { name: "Row 3" })).toBeVisible()
    // The revealed row becomes the active one; Enter inspects its value.
    canvas.getByRole("grid").focus()
    await userEvent.keyboard("{Enter}")
    await expect(onInspect).toHaveBeenCalledWith({ row: 7, column: 0 })
  },
}

export const PageReadFailed: Story = {
  args: {
    resultKey: "story-failed-page",
    rowCount: 40,
    fetchPage: () =>
      Promise.reject(new Error("Batch 3 could not be read from disk.")),
  },
  play: async ({ canvas }) => {
    await expect(
      await canvas.findByText("This page could not be read")
    ).toBeVisible()
    await expect(canvas.getByText("The query was not run again.")).toBeVisible()
  },
}

/** 1 000 columns: only those on screen are drawn, and pages shrink to 20 rows. */
export const ThousandColumns: Story = {
  args: {
    resultKey: "story-1000-columns",
    columns: thousandColumns,
    rowCount: 100_000,
    fetchPage: pagesOf(
      100_000,
      (row, column) =>
        column % 3 === 0 ? String(row * column) : `r${row}c${column}`,
      1000
    ),
  },
  play: async ({ canvas }) => {
    const grid = canvas.getByRole("grid")
    await expect(grid).toHaveAttribute("aria-colcount", "1001")
    await waitFor(() =>
      expect(canvas.getAllByText("r0c1").length).toBeGreaterThan(0)
    )
    await expect(canvas.getAllByRole("columnheader").length).toBeLessThan(60)
    grid.focus()
    await userEvent.keyboard("{End}")
    await waitFor(() =>
      expect(canvas.getByText("metric_0999")).toBeInTheDocument()
    )
  },
}

/** Hostile, RTL, emoji and enormous values stay text, cut once, sized. */
export const HostileAndHugeValues: Story = {
  args: {
    resultKey: "story-odd",
    columns: oddColumns,
    rowCount: 500,
    fetchPage: pagesOf(500, oddCell, oddColumns.length),
  },
  play: async ({ canvas, canvasElement }) => {
    await waitFor(() =>
      expect(
        canvas.getAllByText("<img src=x onerror=alert(0)>").length
      ).toBeGreaterThan(0)
    )
    await expect(canvasElement.querySelector("img")).toBeNull()
    await expect(canvas.getAllByText("50.0 MB").length).toBeGreaterThan(0)
    // One ellipsis at most: the text is cut by CSS, the size says the rest.
    await expect(canvas.queryAllByText(/…$/)).toHaveLength(0)
  },
}

/** An absent value and the text « NULL » never look alike. */
export const NullIsNotTheWordNull: Story = {
  args: {
    resultKey: "story-null",
    columns: oddColumns,
    rowCount: 6,
    fetchPage: pagesOf(6, oddCell, oddColumns.length),
  },
  play: async ({ canvasElement, canvas }) => {
    await waitFor(() =>
      expect(canvas.getAllByText("NULL").length).toBeGreaterThan(0)
    )
    const absent = canvasElement.querySelectorAll("[data-null]")
    await expect(absent.length).toBeGreaterThan(0)
    for (const element of absent) {
      await expect(element).toHaveTextContent("∅ NULL")
    }
    const literal = canvas.getAllByText("NULL")
    for (const element of literal) {
      await expect(element).not.toHaveAttribute("data-null")
    }
  },
}

/** Screen readers follow the active cell through aria-activedescendant. */
export const KeyboardOnly: Story = {
  args: {
    resultKey: "story-keyboard",
    rowCount: 300,
    fetchPage: syntheticPages(300, 0),
  },
  play: async ({ canvas }) => {
    const grid = canvas.getByRole("grid")
    grid.focus()
    await userEvent.keyboard("{ArrowDown}{ArrowDown}{ArrowRight}")
    await waitFor(() => expect(grid).toHaveAttribute("aria-activedescendant"))
    const id = grid.getAttribute("aria-activedescendant") ?? ""
    const cell = document.getElementById(id)
    await expect(cell).toHaveAttribute("role", "gridcell")
    await expect(cell).toHaveAttribute("aria-selected", "true")
    await expect(cell).toHaveAttribute("aria-colindex", "3")
    await expect(cell?.parentElement).toHaveAttribute("aria-rowindex", "4")
    await userEvent.keyboard("{PageDown}")
    await waitFor(() =>
      expect(grid.getAttribute("aria-activedescendant")).not.toBe(id)
    )
  },
}

/** A column is resized from the keyboard, and Home gives its width back. */
export const ResizeFromTheKeyboard: Story = {
  args: {
    resultKey: "story-resize",
    rowCount: 40,
    fetchPage: syntheticPages(40, 0),
  },
  play: async ({ canvas }) => {
    const handle = await canvas.findByRole("separator", {
      name: "Resize column id",
    })
    await waitFor(() =>
      expect(canvas.getAllByText("Acme SA").length).toBeGreaterThan(0)
    )
    const before = Number(handle.getAttribute("aria-valuenow"))
    handle.focus()
    await userEvent.keyboard("{ArrowRight}{ArrowRight}")
    await waitFor(() =>
      expect(Number(handle.getAttribute("aria-valuenow"))).toBe(before + 32)
    )
    await userEvent.keyboard("{Shift>}{ArrowRight}{/Shift}{ArrowLeft}")
    await expect(Number(handle.getAttribute("aria-valuenow"))).toBe(before + 80)
    await userEvent.keyboard("{Home}")
    await expect(Number(handle.getAttribute("aria-valuenow"))).toBe(before)
    // The grid's own arrows did not move while the handle had focus.
    await expect(
      canvas.queryAllByRole("gridcell", { selected: true })
    ).toHaveLength(0)
  },
}

/** The column names the header draws, in the order it draws them. */
const headerOrder = (grid: HTMLElement) =>
  Array.from(
    grid.querySelectorAll('[role="columnheader"] > span:first-child'),
    (name) => name.textContent
  ).filter((name) => name !== "Row number")

/**
 * ⌥⇧← moves the active cell's column: the cell stays on it, the result's
 * own indexes do not change — the moved column's cells still carry their
 * Arrow index in their id — and the move is said aloud.
 */
export const MoveAColumnFromTheKeyboard: Story = {
  args: {
    resultKey: "story-move-keyboard",
    rowCount: 40,
    fetchPage: syntheticPages(40, 0),
  },
  play: async ({ canvas }) => {
    const grid = canvas.getByRole("grid")
    await waitFor(() =>
      expect(canvas.getAllByText("Acme SA").length).toBeGreaterThan(0)
    )
    await expect(grid).toHaveAttribute(
      "aria-keyshortcuts",
      "Alt+Shift+ArrowLeft Alt+Shift+ArrowRight"
    )
    grid.focus()
    await userEvent.keyboard("{ArrowDown}{ArrowRight}")
    await userEvent.keyboard("{Alt>}{Shift>}{ArrowLeft}{/Shift}{/Alt}")
    await waitFor(() =>
      expect(headerOrder(grid).slice(0, 3)).toEqual([
        "customer",
        "id",
        "amount",
      ])
    )
    const id = grid.getAttribute("aria-activedescendant") ?? ""
    await expect(id).toMatch(/-r1-c1$/)
    await expect(document.getElementById(id)).toHaveAttribute(
      "aria-colindex",
      "2"
    )
    await expect(canvas.getByRole("status")).toHaveTextContent(
      "customer moved to position 1 of"
    )
    // Moving is not selecting: one cell stays selected.
    await expect(
      canvas.getAllByRole("gridcell", { selected: true })
    ).toHaveLength(1)
  },
}

/** A header dragged past two others lands after them. */
export const MoveAColumnWithThePointer: Story = {
  args: {
    resultKey: "story-move-pointer",
    rowCount: 40,
    fetchPage: syntheticPages(40, 0),
  },
  play: async ({ canvas }) => {
    const grid = canvas.getByRole("grid")
    await waitFor(() =>
      expect(canvas.getAllByText("Acme SA").length).toBeGreaterThan(0)
    )
    const headers = canvas.getAllByRole("columnheader")
    const [, id, , amount] = headers
    if (!id || !amount) throw new Error("the invoice columns are drawn")
    drag(id, { x: centerOf(amount).x + 4, y: centerOf(amount).y })
    await waitFor(() =>
      expect(headerOrder(grid).slice(0, 3)).toEqual([
        "customer",
        "amount",
        "id",
      ])
    )
    // The release was not a click on a cell.
    await expect(
      canvas.queryAllByRole("gridcell", { selected: true })
    ).toHaveLength(0)
  },
}

/** A slow page shows placeholders, never blank rows or invented values. */
export const SlowPage: Story = {
  args: {
    resultKey: "story-slow",
    rowCount: 5000,
    fetchPage: syntheticPages(5000, 600_000),
  },
  play: async ({ canvasElement }) => {
    await expect(
      canvasElement.querySelectorAll("[data-slot=skeleton]").length
    ).toBeGreaterThan(0)
  },
}

export const Narrow: Story = {
  args: {
    resultKey: "story-narrow",
    rowCount: 2000,
    fetchPage: syntheticPages(2000, 0),
  },
  decorators: [
    (Story) => (
      <div className="h-[480px] w-[420px] border">
        <Story />
      </div>
    ),
  ],
}

export const Empty: Story = {
  args: {
    resultKey: "story-zero",
    rowCount: 0,
    fetchPage: syntheticPages(0, 0),
  },
}

/** Comfortable density: rows follow `--grid-row-height`, never overlap. */
export const ComfortableDensity: Story = {
  args: {
    resultKey: "story-comfortable",
    rowCount: 40,
    fetchPage: syntheticPages(40, 0),
  },
  decorators: [
    (Story) => (
      <div data-density="comfortable" className="h-[560px]">
        <Story />
      </div>
    ),
  ],
  play: async ({ canvas }) => {
    // Row 0 is the header; the first data row follows it.
    await waitFor(() =>
      expect(
        canvas.getAllByRole("row")[1]?.getBoundingClientRect().height
      ).toBe(28)
    )
    // The preset moves the text as well as the rows: a grid stuck at 12 px
    // would make « Comfortable » a row-height setting and nothing else. The
    // gutter reads at the caption size of the same preset — both are tokens,
    // and a token that generated no CSS would fail here rather than silently.
    await expect(getComputedStyle(canvas.getByRole("grid")).fontSize).toBe(
      "14px"
    )
    const gutter = await canvas.findByRole("rowheader", { name: "Row 1" })
    await expect(getComputedStyle(gutter).fontSize).toBe("12px")
  },
}
