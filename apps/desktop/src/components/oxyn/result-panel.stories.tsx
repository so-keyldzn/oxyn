import * as React from "react"
import type { Meta, StoryObj } from "@storybook/react-vite"
import { expect, fn, userEvent, waitFor, within } from "storybook/test"

import { invoiceColumns, syntheticPages } from "./fixtures"
import { ResultPanel } from "./result-panel"
import type { ResultState } from "./result-panel"
import { Button } from "@/components/ui/button"

// The five states of docs/UX-SPEC.md, « États d'une vue ». Each is a story, so
// each is rendered and checked by axe in `make qualite`.
const meta = {
  title: "Oxyn/ResultPanel",
  component: ResultPanel,
  decorators: [
    (Story) => (
      <div className="h-[480px] border">
        <Story />
      </div>
    ),
  ],
  args: {
    state: { status: "initial" },
    fetchPage: syntheticPages(250_000),
    onCancel: fn(),
    onRetry: fn(),
  },
} satisfies Meta<typeof ResultPanel>

export default meta
type Story = StoryObj<typeof meta>

export const Initial: Story = {}

export const Running: Story = {
  args: { state: { status: "running", rows: 12_000, serverCancel: true } },
  play: async ({ canvas, args }) => {
    // « Running » always carries a way to cancel.
    canvas.getByRole("button", { name: "Cancel" }).click()
    await expect(args.onCancel).toHaveBeenCalled()
  },
}

export const RunningWithoutServerCancel: Story = {
  args: { state: { status: "running", rows: 0, serverCancel: false } },
  play: async ({ canvas }) => {
    // The button stays; the promise is withdrawn (docs/UX-SPEC.md, « Annulation »).
    await expect(canvas.getByRole("button", { name: "Cancel" })).toBeVisible()
    await expect(canvas.getByText(/cannot send a cancel request/)).toBeVisible()
  },
}

export const Populated: Story = {
  args: {
    state: {
      status: "populated",
      result: "panel-populated",
      columns: invoiceColumns,
      rows: 250_000,
      complete: true,
      truncated: false,
      cancelled: false,
    },
  },
}

/**
 * Columns hides a column from the grid only: the other headers stay, the
 * footer's export keeps it, and Show all brings it back.
 */
export const HideAColumn: Story = {
  args: {
    state: {
      status: "populated",
      result: "panel-hide-column",
      columns: invoiceColumns,
      rows: 250_000,
      complete: true,
      truncated: false,
      cancelled: false,
    },
  },
  play: async ({ canvas }) => {
    const body = within(document.body)
    const grid = canvas.getByRole("grid")
    await expect(
      within(grid).getByRole("columnheader", { name: /^customer/ })
    ).toBeInTheDocument()
    await expect(grid).toHaveAttribute("aria-colcount", "8")

    await userEvent.click(canvas.getByRole("button", { name: "Columns" }))
    await userEvent.click(
      await body.findByRole("menuitemcheckbox", { name: /customer/ })
    )
    await expect(body.getByText("6 of 7 columns shown")).toBeInTheDocument()
    await waitFor(() =>
      expect(
        within(grid).queryByRole("columnheader", { name: /^customer/ })
      ).toBeNull()
    )
    await expect(grid).toHaveAttribute("aria-colcount", "7")
    await expect(
      within(grid).getByRole("columnheader", { name: /^amount/ })
    ).toBeInTheDocument()

    await userEvent.click(body.getByRole("menuitem", { name: "Show all" }))
    await waitFor(() =>
      expect(
        within(grid).getByRole("columnheader", { name: /^customer/ })
      ).toBeInTheDocument()
    )
    await userEvent.keyboard("{Escape}")
    await waitFor(() => expect(body.queryByRole("menu")).toBeNull())
  },
}

export const Truncated: Story = {
  args: {
    state: {
      status: "populated",
      result: "panel-truncated",
      columns: invoiceColumns,
      rows: 2_000_000,
      complete: false,
      truncated: true,
      cancelled: false,
    },
    fetchPage: syntheticPages(2_000_000),
  },
  play: async ({ canvas }) => {
    // Said under the rows, where the export scope is read.
    await expect(
      canvas.getByText("Truncated · 2,000,000 rows shown, not the whole result")
    ).toBeVisible()
  },
}

export const EmptyResult: Story = {
  args: {
    state: {
      status: "populated",
      result: "panel-empty",
      columns: invoiceColumns,
      rows: 0,
      complete: true,
      truncated: false,
      cancelled: false,
    },
  },
  play: async ({ canvas }) => {
    // Empty is visibly distinct from an error.
    await expect(canvas.getByText("No rows")).toBeVisible()
    await expect(canvas.queryByRole("alert")).toBeNull()
  },
}

export const ServerError: Story = {
  args: {
    state: {
      status: "error",
      message:
        'ERROR:  relation "invoice" does not exist\nLINE 1: SELECT * FROM invoice\n                      ^\nSQLSTATE: 42P01',
      retryable: false,
    },
  },
  play: async ({ canvas }) => {
    // The server's words, code included.
    await expect(canvas.getByText(/42P01/)).toBeVisible()
    await expect(canvas.queryByRole("button", { name: "Run again" })).toBeNull()
  },
}

export const TransientError: Story = {
  args: {
    state: {
      status: "error",
      message: "connection reset by peer",
      retryable: true,
    },
  },
  play: async ({ canvas, args }) => {
    await expect(canvas.getByText(/transient/)).toBeVisible()
    await userEvent.click(canvas.getByRole("button", { name: /Run again/ }))
    await expect(args.onRetry).toHaveBeenCalled()
  },
}

const onEditQuery = fn()

/** The error says where it ran; the server's words stay readable, not red. */
export const ErrorWithContext: Story = {
  args: {
    state: {
      status: "error",
      message:
        'ERROR:  column "totl" does not exist\nLINE 3:   SUM(totl) AS revenue\n              ^\nHINT:  Perhaps you meant to reference the column "invoices.total".\nSQLSTATE: 42703',
      retryable: false,
    },
    context: {
      connectionName: "billing-prod",
      statement:
        "SELECT customer_id,\n  SUM(totl) AS revenue\nFROM invoices GROUP BY customer_id",
    },
    onEditQuery,
  },
  play: async ({ canvas }) => {
    await expect(canvas.getByText("billing-prod")).toBeVisible()
    await expect(canvas.getByText("SELECT customer_id,…")).toBeVisible()
    const message = canvas.getByLabelText("Server message")
    await expect(message).toHaveTextContent("42703")
    await expect(message).toHaveClass("text-foreground")
    await expect(canvas.queryByRole("button", { name: /Run again/ })).toBeNull()
    await userEvent.click(canvas.getByRole("button", { name: /Edit query/ }))
    await expect(onEditQuery).toHaveBeenCalled()
  },
}

/** A policy refusal: the reason, and no promise that retrying helps. */
export const Denied: Story = {
  args: {
    state: {
      status: "error",
      message:
        "Writes are not allowed on billing-prod: the connection is read-only.",
      retryable: false,
    },
    context: {
      connectionName: "billing-prod",
      statement: "DELETE FROM invoices",
    },
  },
  play: async ({ canvas }) => {
    await expect(canvas.getByText(/read-only/)).toBeVisible()
    await expect(canvas.getByText(/will fail the same way/)).toBeVisible()
  },
}

const cancelledState: ResultState = {
  status: "populated",
  result: "panel-cancelled",
  columns: invoiceColumns,
  rows: 3_400,
  complete: false,
  truncated: false,
  cancelled: true,
  elapsedMs: 12_400,
}

export const Cancelled: Story = {
  args: { state: cancelledState, fetchPage: syntheticPages(3_400) },
  play: async ({ canvas }) => {
    await expect(
      canvas.getByText(/Cancelled · 3,400 rows shown, not the whole result/)
    ).toBeVisible()
  },
}

export const CancelledBeforeAnyRow: Story = {
  args: { state: { ...cancelledState, rows: 0 } },
  play: async ({ canvas }) => {
    await expect(canvas.getByText(/cancelled before any row/)).toBeVisible()
  },
}

/** Count, duration, what was not counted, and the export, under the rows. */
export const WithFooter: Story = {
  args: {
    state: {
      status: "populated",
      result: "panel-footer",
      columns: invoiceColumns,
      rows: 200,
      complete: true,
      truncated: false,
      cancelled: false,
      elapsedMs: 84,
    },
    fetchPage: syntheticPages(200, 0),
    footerNote: "Total count not requested",
    footerActions: (
      <Button size="xs" variant="outline">
        Export preview…
      </Button>
    ),
  },
  play: async ({ canvas }) => {
    await expect(canvas.getByText("200 rows shown · 84 ms")).toBeVisible()
    await expect(canvas.getByText("Total count not requested")).toBeVisible()
    await expect(
      canvas.getByRole("button", { name: "Export preview…" })
    ).toBeVisible()
  },
}

/**
 * The text size from the result bar: the same two presets as the settings.
 * Choosing one asks for no page and reruns nothing (docs/UX-SPEC.md,
 * « Lisibilité et hauteur de grille »).
 */
export const TextSize: Story = {
  args: {
    state: {
      status: "populated",
      result: "panel-text-size",
      columns: invoiceColumns,
      rows: 200,
      complete: true,
      truncated: false,
      cancelled: false,
    },
    fetchPage: syntheticPages(200, 0),
    density: { value: "compact", onChange: fn() },
  },
  play: async ({ canvas, args }) => {
    const body = within(document.body)
    await waitFor(() =>
      expect(canvas.getAllByText("Acme SA").length).toBeGreaterThan(0)
    )
    await userEvent.click(
      canvas.getByRole("button", { name: "Text size: Compact" })
    )
    await waitFor(() =>
      expect(
        body.getByRole("menuitemradio", { name: /Compact/ })
      ).toHaveAttribute("aria-checked", "true")
    )
    await userEvent.click(
      body.getByRole("menuitemradio", { name: /Comfortable/ })
    )
    await expect(args.density?.onChange).toHaveBeenCalledWith("comfortable")
    await expect(args.onRetry).not.toHaveBeenCalled()
    await expect(args.onCancel).not.toHaveBeenCalled()
  },
}

/** No grid, no text size: nothing on screen would change. */
export const TextSizeWithoutRows: Story = {
  args: {
    state: {
      status: "populated",
      result: "panel-text-size-empty",
      columns: invoiceColumns,
      rows: 0,
      complete: true,
      truncated: false,
      cancelled: false,
    },
    density: { value: "comfortable", onChange: fn() },
  },
  play: async ({ canvas }) => {
    await expect(canvas.getByText("No rows")).toBeVisible()
    await expect(
      canvas.queryByRole("button", { name: /^Text size/ })
    ).toBeNull()
  },
}

/** The schema is known: rows are readable while the stream still runs. */
function Streaming(props: React.ComponentProps<typeof ResultPanel>) {
  const [rows, setRows] = React.useState(0)
  React.useEffect(() => {
    const timer = setInterval(
      () => setRows((count) => Math.min(count + 700, 5_000)),
      250
    )
    return () => clearInterval(timer)
  }, [])
  const fetchPage = React.useMemo(() => syntheticPages(5_000, 20), [])
  return (
    <ResultPanel
      {...props}
      fetchPage={fetchPage}
      state={{
        status: "running",
        rows,
        serverCancel: true,
        result: "panel-streaming",
        columns: invoiceColumns,
      }}
    />
  )
}

export const RunningWithRows: Story = {
  render: (args) => <Streaming {...args} />,
  play: async ({ canvas, args }) => {
    await waitFor(() => expect(canvas.getByRole("grid")).toBeVisible())
    await waitFor(
      () => expect(canvas.getAllByText("Acme SA").length).toBeGreaterThan(0),
      { timeout: 3000 }
    )
    await waitFor(() =>
      expect(canvas.getByText(/rows received · running/)).toBeVisible()
    )
    // The live region says « Running » once, not every batch.
    await expect(
      canvas
        .getAllByRole("status")
        .some((region) => region.textContent === "Running")
    ).toBe(true)
    await userEvent.click(canvas.getByRole("button", { name: "Cancel" }))
    await expect(args.onCancel).toHaveBeenCalled()
  },
}

export const Narrow: Story = {
  args: { state: cancelledState, fetchPage: syntheticPages(3_400) },
  decorators: [
    (Story) => (
      <div className="h-[420px] w-[420px] border">
        <Story />
      </div>
    ),
  ],
}
