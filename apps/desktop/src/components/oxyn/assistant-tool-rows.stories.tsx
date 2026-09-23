import type { Meta, StoryObj } from "@storybook/react-vite"
import { expect, fn, userEvent } from "storybook/test"

import { AssistantToolRows } from "./assistant-tool-rows"
import { invoiceColumns, syntheticPages } from "./fixtures"
import type { FetchPage } from "./result-grid"
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
 * More rows than the chat reaches: the first hundred, and the whole result one
 * click away, in a tab — never by running the query again.
 */
export const Populated: Story = {
  play: async ({ args, canvas }) => {
    const grid = await canvas.findByRole("grid", {
      name: "Rows the query returned",
    })
    await expect(grid).toBeVisible()
    // The chat grid is bounded: 100 rows and a header, however large the result.
    await expect(grid).toHaveAttribute("aria-rowcount", "101")
    await expect(canvas.getByText(/First 100 rows of 1,200/)).toBeVisible()
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
