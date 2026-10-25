import * as React from "react"
import type { Meta, StoryObj } from "@storybook/react-vite"
import { expect, fn, userEvent, waitFor } from "storybook/test"
import type { Mock, within } from "storybook/test"

import { AssistantToolRows } from "./assistant-tool-rows"
import type { ToolRowsState } from "./assistant-tool-rows"
import { invoiceColumns, syntheticPages } from "./fixtures"
import type { FetchPage } from "./result-grid"
import { Button } from "@/components/ui/button"
import { BackendError } from "@/lib/ipc/client"
import type { Cell } from "@/lib/ipc/types"

const open = {
  status: "open" as const,
  result: "agent-invoices",
  columns: invoiceColumns,
  rows: 1_200,
  truncated: false,
}

const meta = {
  title: "Oxyn/Assistant/ToolRows",
  component: AssistantToolRows,
  decorators: [
    (Story) => (
      <div className="w-[420px] overflow-hidden rounded-lg border bg-card">
        <Story />
      </div>
    ),
  ],
  args: {
    state: open,
    fetchPage: syntheticPages(1_200, 0),
    onOpenAll: fn(),
    onRetry: fn(),
  },
} satisfies Meta<typeof AssistantToolRows>

export default meta
type Story = StoryObj<typeof meta>

/**
 * More rows than one chat page: the first hundred, the next ones a page away,
 * and the whole result one click away, in a tab — never by running the query
 * again.
 */
export const Populated: Story = {
  play: async ({ args, canvas }) => {
    const grid = await canvas.findByRole("grid", {
      name: "Rows the query returned, 1–100 of 1,200",
    })
    await expect(grid).toBeVisible()
    // The chat grid is bounded: 100 rows and a header, however large the result.
    await expect(grid).toHaveAttribute("aria-rowcount", "101")
    await expect(canvas.getByText("Rows 1–100 of 1,200")).toBeVisible()
    await expect(
      canvas.getByText(/shown to you only; the model got the count/)
    ).toBeVisible()
    await userEvent.click(canvas.getByRole("button", { name: /Open all rows/ }))
    await expect(args.onOpenAll).toHaveBeenCalledOnce()
  },
}

/** A handful of rows: the grid is as tall as they are, and counts them all. */
export const FewRows: Story = {
  args: { state: { ...open, rows: 3 }, fetchPage: syntheticPages(3, 0) },
  play: async ({ canvas }) => {
    await expect(await canvas.findByText("Initech")).toBeVisible()
    await expect(canvas.getByText(/^3 rows$/)).toBeVisible()
  },
}

export const Truncated: Story = {
  args: { state: { ...open, truncated: true } },
  play: async ({ canvas }) => {
    await expect(canvas.getByText(/cut at the row limit/)).toBeVisible()
  },
}

export const Loading: Story = {
  args: { state: { status: "loading" } },
  play: async ({ canvas }) => {
    await expect(canvas.getByText(/Reading the rows Oxyn kept/)).toBeVisible()
  },
}

/** Zero rows is an answer, not a failure: no alert, no grid. */
export const Empty: Story = {
  args: { state: { ...open, rows: 0 } },
  play: async ({ canvas }) => {
    await expect(canvas.getByText("The query returned no rows.")).toBeVisible()
    await expect(canvas.queryByRole("alert")).toBeNull()
    await expect(canvas.queryByRole("grid")).toBeNull()
  },
}

/** Retention released the buffer: said, and nothing offers to rerun it. */
export const Expired: Story = {
  args: { state: { status: "expired" } },
  play: async ({ canvas }) => {
    await expect(canvas.getByText(/Result no longer available/)).toBeVisible()
    await expect(canvas.queryByRole("button")).toBeNull()
  },
}

/** The backend's words, whole; « Try again » only because it said so. */
export const Failed: Story = {
  args: {
    state: {
      status: "error",
      message:
        "result 7f3a… belongs to another connection; nothing was read (OpenRetainedResult)",
      retryable: true,
    },
  },
  play: async ({ args, canvas }) => {
    await expect(
      canvas.getByText(/belongs to another connection/)
    ).toBeVisible()
    await userEvent.click(canvas.getByRole("button", { name: "Try again" }))
    await expect(args.onRetry).toHaveBeenCalledOnce()
  },
}

/** A page answered from rows written out, as `read_result_page` would. */
function pagesOf(rows: Array<Array<Cell>>): FetchPage {
  return (offset, limit) =>
    Promise.resolve({
      type: "page",
      offset,
      rows: rows.slice(offset, offset + limit),
      totalRows: rows.length,
      complete: true,
    })
}

// Rust groups digits by U+00A0, spelled by its code: written out, it is
// invisible in the source.
const GROUP = String.fromCharCode(0xa0)

const byStatus = {
  columns: [
    { name: "status", dataType: "Utf8", nullable: false },
    { name: "orders", dataType: "Int64", nullable: false },
    { name: "revenue", dataType: "Decimal128(12, 2)", nullable: true },
  ],
  rows: [
    ["shipped", `1${GROUP}204`, `48${GROUP}210.50`],
    ["pending", "312", `9${GROUP}874.00`],
    ["cancelled", "58", null],
    ["returned", "41", `1${GROUP}230.25`],
  ] satisfies Array<Array<Cell>>,
}

/** A category and numbers: bars, drawn from the rows the grid shows. */
export const Charted: Story = {
  args: {
    state: { ...open, columns: byStatus.columns, rows: byStatus.rows.length },
    fetchPage: pagesOf(byStatus.rows),
  },
  play: async ({ canvas }) => {
    const chart = canvas.getByRole("button", { name: "Chart" })
    await expect(chart).toHaveAttribute("aria-pressed", "false")
    await userEvent.click(chart)
    await expect(chart).toHaveAttribute("aria-pressed", "true")
    await expect(
      await canvas.findByText(
        "orders, revenue by status, in the order the query returned"
      )
    ).toBeVisible()
    await expect(canvas.queryByRole("grid")).toBeNull()
    await userEvent.click(chart)
    await expect(
      await canvas.findByRole("grid", { name: "Rows the query returned" })
    ).toBeVisible()
  },
}

const byDay = {
  columns: [
    { name: "day", dataType: "Date32", nullable: false },
    { name: "signups", dataType: "Int32", nullable: false },
    { name: "note", dataType: "Decimal128(5, 2)", nullable: true },
  ],
  rows: Array.from({ length: 14 }, (_, index): Array<Cell> => [
    `2026-09-${String(index + 1).padStart(2, "0")}`,
    String(40 + ((index * 17) % 23)),
    // A value cut by the backend is not a number: the column is left out.
    index === 3 ? { text: "12.5", fullBytes: 900 } : "1.00",
  ]),
}

/** A date axis draws a line; a column that is not plainly numeric is named. */
export const ChartedByDate: Story = {
  args: {
    state: { ...open, columns: byDay.columns, rows: byDay.rows.length },
    fetchPage: pagesOf(byDay.rows),
  },
  play: async ({ canvas }) => {
    await userEvent.click(canvas.getByRole("button", { name: "Chart" }))
    await expect(
      await canvas.findByText(/left out, not plainly numeric: note/)
    ).toBeVisible()
  },
}

/** The columns allow a chart, the values do not: said, nothing drawn. */
export const NothingToChart: Story = {
  args: {
    state: {
      ...open,
      columns: [
        { name: "label", dataType: "Utf8", nullable: false },
        { name: "ratio", dataType: "Float64", nullable: true },
      ],
      rows: 2,
    },
    fetchPage: pagesOf([
      ["a", "NaN"],
      ["b", null],
    ]),
  },
  play: async ({ canvas }) => {
    await userEvent.click(canvas.getByRole("button", { name: "Chart" }))
    await expect(await canvas.findByText(/Nothing to chart/)).toBeVisible()
  },
}

/** No numeric column: no Chart button, rather than one that draws nothing. */
export const NoChartOffered: Story = {
  args: {
    state: {
      ...open,
      columns: [
        { name: "name", dataType: "Utf8", nullable: false },
        { name: "email", dataType: "Utf8", nullable: false },
      ],
      rows: 1,
    },
    fetchPage: pagesOf([["Ada", "ada@example.com"]]),
  },
  play: async ({ canvas }) => {
    await expect(canvas.queryByRole("button", { name: "Chart" })).toBeNull()
  },
}

export const ChartedLight: Story = {
  ...Charted,
  globals: { theme: "light" },
}

export const FailedForGood: Story = {
  args: {
    state: {
      status: "error",
      message: "the executor refused to open this result",
      retryable: false,
    },
  },
  play: async ({ canvas }) => {
    await expect(canvas.queryByRole("button", { name: "Try again" })).toBeNull()
  },
}

const BIG = 250_000

/** Every read a paged story makes, kept to check none leaves the rows held. */
const bigPages = fn(syntheticPages(BIG, 0))
const shortPages = fn(syntheticPages(250, 0))

/** No read ever asked for a row the retained result does not hold. */
async function readInside(spy: Mock<FetchPage>, rows: number) {
  await expect(spy).toHaveBeenCalled()
  for (const [offset, limit] of spy.mock.calls) {
    await expect(offset).toBeGreaterThanOrEqual(0)
    await expect(offset + limit).toBeLessThanOrEqual(rows)
  }
}

/** A pager button stays in the tab order at a bound, inert, and says why. */
async function inert(button: HTMLElement, reason: string) {
  await expect(button).toHaveAttribute("aria-disabled", "true")
  await expect(button).toHaveAccessibleDescription(reason)
}

async function live(button: HTMLElement) {
  await expect(button.getAttribute("aria-disabled")).not.toBe("true")
}

const paged = { state: { ...open, rows: BIG }, fetchPage: bigPages }

/** The first of 2,500 pages: nothing before it, said rather than hidden. */
export const FirstPage: Story = {
  args: paged,
  play: async ({ canvas }) => {
    await expect(
      await canvas.findByRole("grid", {
        name: "Rows the query returned, 1–100 of 250,000",
      })
    ).toHaveAttribute("aria-rowcount", "101")
    await expect(canvas.getByText("Rows 1–100 of 250,000")).toBeVisible()
    await inert(
      canvas.getByRole("button", { name: "Previous page" }),
      "This is the first page"
    )
    await live(canvas.getByRole("button", { name: "Next page" }))
    await readInside(bigPages, BIG)
  },
}

/**
 * Next reads rows 101–200 from the buffer, Previous comes back: the range is
 * announced once, focus stays on the button, and the gutter numbers the
 * result's rows.
 */
export const PagesForwardAndBack: Story = {
  args: paged,
  play: async ({ args, canvas }) => {
    await canvas.findByRole("rowheader", { name: "Row 1" })
    const next = canvas.getByRole("button", { name: "Next page" })
    await userEvent.click(next)
    await expect(canvas.getByText("Rows 101–200 of 250,000")).toBeVisible()
    await expect(
      canvas.getByText("Showing rows 101–200 of 250,000")
    ).toBeInTheDocument()
    await expect(
      await canvas.findByRole("rowheader", { name: "Row 101" })
    ).toBeVisible()
    await expect(
      canvas.getByRole("grid", {
        name: "Rows the query returned, 101–200 of 250,000",
      })
    ).toBeVisible()
    await expect(next).toHaveFocus()
    await expect(bigPages.mock.calls.map(([offset]) => offset)).toContain(100)

    const previous = canvas.getByRole("button", { name: "Previous page" })
    await userEvent.click(previous)
    await expect(canvas.getByText("Rows 1–100 of 250,000")).toBeVisible()
    await expect(
      canvas.getByText("Showing rows 1–100 of 250,000")
    ).toBeInTheDocument()
    await expect(previous).toHaveFocus()
    await inert(previous, "This is the first page")

    // Paging opens nothing: the whole result is still one click away.
    await userEvent.click(canvas.getByRole("button", { name: /Open all rows/ }))
    await expect(args.onOpenAll).toHaveBeenCalledOnce()
    await readInside(bigPages, BIG)
  },
}

/** Between the bounds, both ways are open. */
export const MiddlePage: Story = {
  args: paged,
  play: async ({ canvas }) => {
    const next = canvas.getByRole("button", { name: "Next page" })
    await userEvent.click(next)
    await userEvent.click(next)
    await expect(canvas.getByText("Rows 201–300 of 250,000")).toBeVisible()
    await expect(
      await canvas.findByRole("rowheader", { name: "Row 201" })
    ).toBeVisible()
    await live(canvas.getByRole("button", { name: "Previous page" }))
    await live(next)
    await readInside(bigPages, BIG)
  },
}

/** The last page holds what is left — 50 rows — and nothing after it. */
export const LastShortPage: Story = {
  args: { state: { ...open, rows: 250 }, fetchPage: shortPages },
  play: async ({ canvas }) => {
    const next = canvas.getByRole("button", { name: "Next page" })
    await userEvent.click(next)
    await userEvent.click(next)
    await expect(canvas.getByText("Rows 201–250 of 250")).toBeVisible()
    await expect(
      canvas.getByRole("grid", {
        name: "Rows the query returned, 201–250 of 250",
      })
    ).toHaveAttribute("aria-rowcount", "51")
    await inert(next, "This is the last page")
    await expect(next).toHaveFocus()
    await userEvent.click(next)
    await expect(canvas.getByText("Rows 201–250 of 250")).toBeVisible()
    await readInside(shortPages, 250)
  },
}

/** A result that fits one page has no pager: the footer counts its rows. */
export const SinglePage: Story = {
  args: { state: { ...open, rows: 100 }, fetchPage: syntheticPages(100, 0) },
  play: async ({ canvas }) => {
    await expect(
      await canvas.findByRole("grid", { name: "Rows the query returned" })
    ).toHaveAttribute("aria-rowcount", "101")
    await expect(canvas.getByText(/^100 rows$/)).toBeVisible()
    await expect(
      canvas.queryByRole("button", { name: "Previous page" })
    ).toBeNull()
    await expect(canvas.queryByRole("button", { name: "Next page" })).toBeNull()
  },
}

/** Another tool call under the same view: a new result starts on its first page. */
function AnotherCall(args: React.ComponentProps<typeof AssistantToolRows>) {
  const [call, setCall] = React.useState(0)
  const state: ToolRowsState = {
    ...open,
    result: `agent-call-${call}`,
    rows: call === 0 ? BIG : 300,
  }
  return (
    <div className="flex flex-col">
      <Button
        size="xs"
        variant="outline"
        className="m-2 self-start"
        onClick={() => setCall((current) => current + 1)}
      >
        Next tool call
      </Button>
      <AssistantToolRows {...args} state={state} />
    </div>
  )
}

export const ResultChanges: Story = {
  args: paged,
  render: (args) => <AnotherCall {...args} />,
  play: async ({ canvas }) => {
    const next = canvas.getByRole("button", { name: "Next page" })
    await userEvent.click(next)
    await userEvent.click(next)
    await expect(canvas.getByText("Rows 201–300 of 250,000")).toBeVisible()
    await userEvent.click(
      canvas.getByRole("button", { name: "Next tool call" })
    )
    await expect(canvas.getByText("Rows 1–100 of 300")).toBeVisible()
    await inert(
      canvas.getByRole("button", { name: "Previous page" }),
      "This is the first page"
    )
    await expect(
      await canvas.findByRole("rowheader", { name: "Row 1" })
    ).toBeVisible()
  },
}

/** Retention released the buffer between two pages: said, nothing rerun. */
const releasedAfterFirst = fn<FetchPage>((offset, limit) =>
  offset < 100
    ? syntheticPages(BIG, 0)(offset, limit)
    : Promise.resolve({ type: "expired" })
)

const RELEASED =
  "Result no longer available: Oxyn released these rows. Nothing is rerun to bring them back."

/**
 * Chart and Open all rows stay where they were, inert, in the tab order, and
 * say why: the rows they would work on are gone, and nothing reruns them.
 */
async function withheld(button: HTMLElement) {
  await expect(button).toHaveAttribute("aria-disabled", "true")
  await expect(button).toHaveAccessibleDescription(RELEASED)
}

export const ExpiredWhilePaging: Story = {
  args: { ...paged, fetchPage: releasedAfterFirst },
  play: async ({ args, canvas }) => {
    await canvas.findByRole("rowheader", { name: "Row 1" })
    const next = canvas.getByRole("button", { name: "Next page" })
    await userEvent.click(next)
    await expect(await canvas.findByText(RELEASED)).toBeVisible()
    // Said once, politely, without taking the focus.
    await expect(canvas.getByRole("status")).toHaveTextContent(RELEASED)
    await expect(canvas.queryByRole("grid")).toBeNull()
    await inert(next, "Result no longer available")
    await inert(
      canvas.getByRole("button", { name: "Previous page" }),
      "Result no longer available"
    )
    await expect(next).toHaveFocus()
    // Where the reader was, not a promise that the rows are still there.
    await expect(canvas.getByText("Rows 101–200 of 250,000")).toBeVisible()
    const reads = releasedAfterFirst.mock.calls.length

    const chart = canvas.getByRole("button", { name: "Chart" })
    const openAll = canvas.getByRole("button", { name: /Open all rows/ })
    await withheld(chart)
    await withheld(openAll)
    await userEvent.tab()
    await expect(chart).toHaveFocus()
    await userEvent.tab()
    await expect(openAll).toHaveFocus()

    await userEvent.click(openAll)
    await userEvent.keyboard("{Enter}")
    await userEvent.click(chart)
    await expect(args.onOpenAll).not.toHaveBeenCalled()
    await expect(chart).toHaveAttribute("aria-pressed", "false")
    await expect(canvas.queryByRole("button", { name: "Try again" })).toBeNull()
    await expect(releasedAfterFirst.mock.calls.length).toBe(reads)
    await readInside(releasedAfterFirst, BIG)
  },
}

/** The second page fails until the play lets it through. */
const healed = { now: false }
const flakySecondPage = fn<FetchPage>((offset, limit) =>
  offset < 100 || healed.now
    ? syntheticPages(BIG, 0)(offset, limit)
    : Promise.reject(
        new BackendError({
          message: "reading the result: spill file busy (os error 35)",
          retryable: true,
          missingModule: null,
        })
      )
)

/** A page read failed: the backend's words, and « Try again » since it allows it. */
export const PageReadFails: Story = {
  args: { ...paged, fetchPage: flakySecondPage },
  play: async ({ canvas }) => {
    healed.now = false
    await canvas.findByRole("rowheader", { name: "Row 1" })
    await userEvent.click(canvas.getByRole("button", { name: "Next page" }))
    await expect(
      await canvas.findByText(
        "reading the result: spill file busy (os error 35)"
      )
    ).toBeVisible()
    await expect(
      canvas.getByText("The rows could not be read. Nothing was rerun.")
    ).toBeVisible()
    healed.now = true
    await userEvent.click(canvas.getByRole("button", { name: "Try again" }))
    await expect(
      await canvas.findByRole("rowheader", { name: "Row 101" })
    ).toBeVisible()
    await expect(canvas.getByText("Rows 101–200 of 250,000")).toBeVisible()
    await readInside(flakySecondPage, BIG)
  },
}

const brokenSecondPage = fn<FetchPage>((offset, limit) =>
  offset < 100
    ? syntheticPages(BIG, 0)(offset, limit)
    : Promise.reject(
        new BackendError({
          message: "A batch of this result is missing",
          retryable: false,
          missingModule: null,
        })
      )
)

/** Not retryable: the message alone, no button that would fail again. */
export const PageReadFailsForGood: Story = {
  args: { ...paged, fetchPage: brokenSecondPage },
  play: async ({ canvas }) => {
    await canvas.findByRole("rowheader", { name: "Row 1" })
    await userEvent.click(canvas.getByRole("button", { name: "Next page" }))
    await expect(
      await canvas.findByText("A batch of this result is missing")
    ).toBeVisible()
    await expect(canvas.queryByRole("button", { name: "Try again" })).toBeNull()
    // Going back is still a read of the buffer, and it still works.
    await userEvent.click(canvas.getByRole("button", { name: "Previous page" }))
    await expect(
      await canvas.findByRole("rowheader", { name: "Row 1" })
    ).toBeVisible()
  },
}

const slowSecondPage = fn<FetchPage>((offset, limit) =>
  offset < 100
    ? syntheticPages(BIG, 0)(offset, limit)
    : new Promise(() => undefined)
)

/** The next page is on its way: the grid says it is busy, cells wait drawn. */
export const LoadingPage: Story = {
  args: { ...paged, fetchPage: slowSecondPage },
  play: async ({ canvas, canvasElement }) => {
    await canvas.findByRole("rowheader", { name: "Row 1" })
    await userEvent.click(canvas.getByRole("button", { name: "Next page" }))
    await expect(canvas.getByText("Rows 101–200 of 250,000")).toBeVisible()
    await waitFor(() =>
      expect(canvasElement.querySelector('[aria-busy="true"]')).not.toBeNull()
    )
    await expect(
      canvas.getByRole("grid", {
        name: "Rows the query returned, 101–200 of 250,000",
      })
    ).toBeVisible()
    await readInside(slowSecondPage, BIG)
  },
}

/** The panel at its narrowest: the footer wraps, the pager stays whole. */
export const Narrow: Story = {
  args: paged,
  decorators: [
    (Story) => (
      <div className="w-[280px]">
        <Story />
      </div>
    ),
  ],
  play: async ({ canvas }) => {
    await expect(canvas.getByText("Rows 1–100 of 250,000")).toBeVisible()
    await expect(
      canvas.getByRole("button", { name: "Next page" })
    ).toBeVisible()
    await expect(
      canvas.getByRole("button", { name: "Previous page" })
    ).toBeVisible()
  },
}

export const PagedLight: Story = {
  ...PagesForwardAndBack,
  globals: { theme: "light" },
}

const manyStatuses = Array.from({ length: 250 }, (_, index): Array<Cell> => [
  `batch ${index + 1}`,
  String(10 + (index % 17)),
  null,
])

/** The chart draws the page shown, and says which rows it drew. */
export const ChartedPage: Story = {
  args: {
    state: { ...open, columns: byStatus.columns, rows: manyStatuses.length },
    fetchPage: pagesOf(manyStatuses),
  },
  play: async ({ canvas }) => {
    await userEvent.click(canvas.getByRole("button", { name: "Next page" }))
    await userEvent.click(canvas.getByRole("button", { name: "Chart" }))
    await expect(
      await canvas.findByText("Drawn from the page shown: rows 101–200 of 250.")
    ).toBeVisible()
  },
}

const chartReleasedAfterFirst = fn<FetchPage>((offset, limit) =>
  offset < 100
    ? pagesOf(manyStatuses)(offset, limit)
    : Promise.resolve({ type: "expired" })
)

/** The chart was drawn when the rows went: the message takes its place. */
export const ExpiredWhileCharted: Story = {
  args: {
    state: { ...open, columns: byStatus.columns, rows: manyStatuses.length },
    fetchPage: chartReleasedAfterFirst,
  },
  play: async ({ args, canvas }) => {
    const chart = canvas.getByRole("button", { name: "Chart" })
    await userEvent.click(chart)
    await expect(
      await canvas.findByText("Drawn from the page shown: rows 1–100 of 250.")
    ).toBeVisible()
    await userEvent.click(canvas.getByRole("button", { name: "Next page" }))
    await expect(await canvas.findByText(RELEASED)).toBeVisible()
    await expect(canvas.queryByText(/Drawn from the page shown/)).toBeNull()
    await expect(canvas.queryByRole("grid")).toBeNull()
    await withheld(chart)
    await withheld(canvas.getByRole("button", { name: /Open all rows/ }))
    const reads = chartReleasedAfterFirst.mock.calls.length
    // Neither going back to the grid nor opening the result reads anything.
    await userEvent.click(chart)
    await userEvent.click(canvas.getByRole("button", { name: /Open all rows/ }))
    await expect(canvas.queryByRole("grid")).toBeNull()
    await expect(args.onOpenAll).not.toHaveBeenCalled()
    await expect(chartReleasedAfterFirst.mock.calls.length).toBe(reads)
  },
}

/** The column names the header draws, in the order it draws them. */
const headerOrder = (grid: HTMLElement) =>
  Array.from(
    grid.querySelectorAll('[role="columnheader"] > span:first-child'),
    (name) => name.textContent
  ).filter((name) => name !== "Row number")

type Canvas = ReturnType<typeof within>

const handleOf = (canvas: Canvas, column: string) =>
  canvas.findByRole("separator", { name: `Resize column ${column}` })

const widthOf = async (canvas: Canvas, column: string) =>
  Number((await handleOf(canvas, column)).getAttribute("aria-valuenow"))

/**
 * On the page shown: `customer` moved first from the keyboard, then widened
 * from its handle. Returns its width before and after.
 */
async function arrange(canvas: Canvas) {
  await waitFor(() =>
    expect(canvas.getAllByText("Acme SA").length).toBeGreaterThan(0)
  )
  const grid = canvas.getByRole("grid")
  grid.focus()
  await userEvent.keyboard("{ArrowDown}{ArrowRight}")
  await userEvent.keyboard("{Alt>}{Shift>}{ArrowLeft}{/Shift}{/Alt}")
  await waitFor(() =>
    expect(headerOrder(grid).slice(0, 2)).toEqual(["customer", "id"])
  )
  const before = await widthOf(canvas, "customer")
  ;(await handleOf(canvas, "customer")).focus()
  await userEvent.keyboard("{Shift>}{ArrowRight}{ArrowRight}{/Shift}")
  await waitFor(async () =>
    expect(await widthOf(canvas, "customer")).toBeGreaterThan(before)
  )
  return { before, widened: await widthOf(canvas, "customer") }
}

async function arranged(canvas: Canvas, width: number) {
  await waitFor(() =>
    expect(headerOrder(canvas.getByRole("grid")).slice(0, 2)).toEqual([
      "customer",
      "id",
    ])
  )
  await expect(await widthOf(canvas, "customer")).toBe(width)
}

async function byDefault(canvas: Canvas, width: number) {
  await waitFor(() =>
    expect(headerOrder(canvas.getByRole("grid")).slice(0, 2)).toEqual([
      "id",
      "customer",
    ])
  )
  await waitFor(async () =>
    expect(await widthOf(canvas, "customer")).toBe(width)
  )
}

/**
 * A column widened and another order chosen on the first page hold on the
 * next one and on the way back: they are the result's, not the page's.
 * Selection still starts clean on each page.
 */
export const LayoutKeptAcrossPages: Story = {
  args: paged,
  play: async ({ canvas }) => {
    await canvas.findByRole("rowheader", { name: "Row 1" })
    const { widened } = await arrange(canvas)

    await userEvent.click(canvas.getByRole("button", { name: "Next page" }))
    await canvas.findByRole("rowheader", { name: "Row 101" })
    await arranged(canvas, widened)
    await expect(
      canvas.queryAllByRole("gridcell", { selected: true })
    ).toHaveLength(0)

    await userEvent.click(canvas.getByRole("button", { name: "Previous page" }))
    await canvas.findByRole("rowheader", { name: "Row 1" })
    await arranged(canvas, widened)
    await expect(
      canvas.queryAllByRole("gridcell", { selected: true })
    ).toHaveLength(0)
  },
}

/** Another result brings other columns: widths and order start over. */
export const LayoutResetsOnAnotherResult: Story = {
  args: paged,
  render: (args) => <AnotherCall {...args} />,
  play: async ({ canvas }) => {
    await canvas.findByRole("rowheader", { name: "Row 1" })
    const { before } = await arrange(canvas)
    await userEvent.click(canvas.getByRole("button", { name: "Next page" }))
    await canvas.findByRole("rowheader", { name: "Row 101" })
    await userEvent.click(
      canvas.getByRole("button", { name: "Next tool call" })
    )
    await expect(canvas.getByText("Rows 1–100 of 300")).toBeVisible()
    await byDefault(canvas, before)
  },
}

/** The same result answering other columns: the page stays, the layout goes. */
function OtherColumns(args: React.ComponentProps<typeof AssistantToolRows>) {
  const [changed, setChanged] = React.useState(false)
  const state: ToolRowsState = {
    ...open,
    rows: BIG,
    columns: changed
      ? invoiceColumns.map((column) =>
          column.name === "notes"
            ? { ...column, dataType: "LargeUtf8" }
            : column
        )
      : invoiceColumns,
  }
  return (
    <div className="flex flex-col">
      <Button
        size="xs"
        variant="outline"
        className="m-2 self-start"
        onClick={() => setChanged(true)}
      >
        Other columns
      </Button>
      <AssistantToolRows {...args} state={state} />
    </div>
  )
}

export const LayoutResetsOnOtherColumns: Story = {
  args: paged,
  render: (args) => <OtherColumns {...args} />,
  play: async ({ canvas }) => {
    await userEvent.click(canvas.getByRole("button", { name: "Next page" }))
    await canvas.findByRole("rowheader", { name: "Row 101" })
    const { before } = await arrange(canvas)
    await userEvent.click(canvas.getByRole("button", { name: "Other columns" }))
    await expect(canvas.getByText("Rows 101–200 of 250,000")).toBeVisible()
    await canvas.findByRole("rowheader", { name: "Row 101" })
    await byDefault(canvas, before)
  },
}
