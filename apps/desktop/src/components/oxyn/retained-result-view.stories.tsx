import type { Meta, StoryObj } from "@storybook/react-vite"
import { expect, fn, userEvent } from "storybook/test"

import { ExportMenuView } from "./export-menu"
import { invoiceColumns, syntheticPages } from "./fixtures"
import { RetainedResultView, retainedExportable } from "./retained-result-view"
import type { ExportFormatChoice } from "@/lib/ipc/results"

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
    destination: "billing replica",
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

const formats: Array<ExportFormatChoice> = [
  { format: "csv", label: "CSV", extension: "csv", supported: true },
  { format: "json", label: "JSON", extension: "json", supported: true },
]

/** The export menu under the grid, as the workspace tab wires it. */
function exportFor(state: typeof open) {
  return (
    <ExportMenuView
      formats={formats}
      formatsFailed={false}
      exportable={retainedExportable(state)}
      reason="The run did not end cleanly: these rows may not be the whole result."
      state={{ status: "idle" }}
      onExport={fn()}
      onCancel={fn()}
    />
  )
}

/** A whole, successful result exports the rows already retained. */
export const ExportOffered: Story = {
  args: { footerActions: exportFor(open) },
  play: async ({ canvas }) => {
    const trigger = canvas.getByRole("button", { name: "Export" })
    await expect(trigger).not.toHaveAttribute("aria-disabled", "true")
    await expect(trigger).not.toHaveAccessibleDescription(/Unavailable/)
  },
}

/** Incomplete rows stay readable, but are never offered as a whole export. */
export const ExportRefusedWhenIncomplete: Story = {
  args: {
    state: { ...open, complete: false },
    footerActions: exportFor({ ...open, complete: false }),
  },
  play: async ({ canvas }) => {
    const trigger = canvas.getByRole("button", { name: /Export/ })
    await expect(trigger).toHaveAttribute("aria-disabled", "true")
    await expect(trigger).toHaveAccessibleDescription(
      /Unavailable: The run did not end cleanly/
    )
  },
}

/** Released rows are not brought back by running the query again. */
export const Expired: Story = {
  args: { state: { status: "expired" } },
  play: async ({ canvas, args }) => {
    await expect(canvas.getByText(/No query was rerun/)).toBeVisible()
    await expect(canvas.queryByRole("button", { name: /Run/ })).toBeNull()
    // The destination is named before the click: the copy opens here.
    await userEvent.click(
      canvas.getByRole("button", {
        name: "Open a copy of the statement in billing replica",
      })
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
