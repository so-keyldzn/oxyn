import type { Meta, StoryObj } from "@storybook/react-vite"
import { expect, fn, userEvent } from "storybook/test"

import { AssistantToolRows } from "./assistant-tool-rows"
import { invoiceColumns, syntheticPages } from "./fixtures"

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
