import type { Meta, StoryObj } from "@storybook/react-vite"
import { expect, fn, userEvent } from "storybook/test"

import { invoiceColumns, syntheticPages } from "./fixtures"
import { RetainedResultView } from "./retained-result-view"

const open = {
  status: "open" as const,
  result: "retained-invoices",
  columns: invoiceColumns,
  rows: 1_200,
  complete: true,
  truncated: false,
}

const meta = {
  title: "Oxyn/RetainedResultView",
  component: RetainedResultView,
  decorators: [
    (Story) => (
      <div className="h-[420px] border">
        <Story />
      </div>
    ),
  ],
  args: {
    state: open,
    fetchPage: syntheticPages(1_200),
    onRetry: fn(),
    onOpenCopy: fn(),
  },
} satisfies Meta<typeof RetainedResultView>

export default meta
type Story = StoryObj<typeof meta>

export const Opened: Story = {
  play: async ({ canvas }) => {
    await expect(canvas.getByText(/do not rerun the query/)).toBeVisible()
  },
}

export const Loading: Story = {
  args: { state: { status: "loading" } },
}

/** The run did not end cleanly: rows for inspection, no export. */
export const Incomplete: Story = {
  args: { state: { ...open, complete: false } },
  play: async ({ canvas }) => {
    await expect(
      canvas.getByText(/Incomplete or uncertain execution/)
    ).toBeVisible()
  },
}

export const Truncated: Story = {
  args: { state: { ...open, truncated: true } },
}

/** Released rows are not brought back by running the query again. */
export const Expired: Story = {
  args: { state: { status: "expired" } },
  play: async ({ canvas, args }) => {
    await expect(canvas.getByText(/No query was rerun/)).toBeVisible()
    await expect(canvas.queryByRole("button", { name: /Run/ })).toBeNull()
    await userEvent.click(
      canvas.getByRole("button", { name: "Open the statement in a console" })
    )
    await expect(args.onOpenCopy).toHaveBeenCalled()
  },
}

export const RetryableError: Story = {
  args: {
    state: {
      status: "error",
      message: "the backend is busy closing sessions",
      retryable: true,
    },
  },
  play: async ({ canvas, args }) => {
    await userEvent.click(canvas.getByRole("button", { name: "Try again" }))
    await expect(args.onRetry).toHaveBeenCalled()
  },
}

/** A definitive refusal offers no retry. */
export const Error: Story = {
  args: {
    state: {
      status: "error",
      message: "this connection is no longer in the workspace",
      retryable: false,
    },
  },
  play: async ({ canvas }) => {
    await expect(canvas.queryByRole("button", { name: "Try again" })).toBeNull()
  },
}
