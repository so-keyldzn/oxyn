import type { Meta, StoryObj } from "@storybook/react-vite"
import { expect, fn, screen, userEvent, waitFor } from "storybook/test"

import { expectContainedInFrame, openFrame } from "./frame-overflow"
import { LibraryPanel } from "./library-panel"
import type { DocumentEntry, HistoryRow } from "@/lib/ipc/library"

const CURRENT = "018f0000-0000-7000-8000-000000000001"

const history: Array<HistoryRow> = [
  {
    id: 12,
    at: "2026-09-15T09:42:10+00:00",
    connectionName: "billing replica",
    preview:
      "SELECT c.name, i.amount FROM invoices AS i JOIN customers AS c ON c.id = i.customer_id",
    status: "succeeded",
    durationMs: 84,
    rows: 112,
    needsInspection: false,
    reconciled: false,
    connection: CURRENT,
    result: "018f0000-0000-7000-8000-00000000aaaa",
    fromAgent: false,
  },
  {
    id: 11,
    at: "2026-09-15T09:40:02+00:00",
    connectionName: "billing primary",
    preview: "UPDATE invoices SET paid_at = now() WHERE id = $1",
    status: "failed",
    durationMs: 30000,
    rows: null,
    needsInspection: true,
    reconciled: false,
    connection: "018f0000-0000-7000-8000-000000000002",
    result: "018f0000-0000-7000-8000-00000000bbbb",
    fromAgent: true,
  },
]

const saved: Array<DocumentEntry> = [
  {
    id: "doc-1",
    title: "unpaid invoices.sql",
    connection: CURRENT,
    connectionName: "billing replica",
    updatedAt: "2026-09-14T17:00:00+00:00",
    isSaved: true,
    isOpen: false,
    hasChanges: false,
    fromAgent: false,
  },
  {
    id: "doc-2",
    title: "churn by month.sql",
    connection: null,
    connectionName: null,
    updatedAt: "2026-09-12T08:00:00+00:00",
    isSaved: true,
    isOpen: false,
    hasChanges: true,
    fromAgent: true,
  },
]

const meta = {
  title: "Oxyn/LibraryPanel",
  component: LibraryPanel,
  decorators: [
    (Story) => (
      <div className="flex h-[640px] w-[300px] flex-col border">
        <Story />
      </div>
    ),
  ],
  args: {
    view: "history",
    onViewChange: fn(),
    search: "",
    onSearchChange: fn(),
    filters: { connection: "", days: 7, status: "all" },
    onFiltersChange: fn(),
    connections: [
      { value: CURRENT, label: "billing replica" },
      { value: "gone", label: "old warehouse · not in this workspace" },
    ],
    state: { status: "history", entries: history },
    hasPrevious: false,
    hasNext: true,
    onPrevious: fn(),
    onNext: fn(),
    searching: false,
    onCancel: fn(),
    onRefresh: fn(),
    currentConnection: CURRENT,
    currentConnectionName: "billing replica",
    onInspectHistory: fn(),
    onInspectSaved: fn(),
    onOpenHistory: fn(),
    onOpenResult: fn(),
    onReconcile: fn(),
    onOpenSaved: fn(),
    onResumeSaved: fn(),
    onDeleteSaved: fn(),
  },
} satisfies Meta<typeof LibraryPanel>

export default meta
type Story = StoryObj<typeof meta>

/** An ambiguous write offers inspection only: no copy, no replay. */
export const History: Story = {
  play: async ({ canvas, args }) => {
    const copies = canvas.getAllByRole("button", { name: /Open copy/ })
    await expect(copies).toHaveLength(1)
    await userEvent.click(copies[0]!)
    await expect(args.onOpenHistory).toHaveBeenCalledWith(history[0])
    await expect(canvas.getByText("Needs inspection")).toBeVisible()
    // The agent's statement says so in the list, before any copy.
    await expect(canvas.getAllByText("AI ·")).toHaveLength(1)
    await expect(canvas.queryByText(new RegExp(CURRENT))).toBeNull()
  },
}

/**
 * Selecting an entry asks for its full text, read only — the write that needs
 * inspection included, although it offers no copy. Nothing opens or runs.
 */
export const SelectReadsTheFullText: Story = {
  play: async ({ canvas, args }) => {
    await userEvent.click(
      canvas.getByRole("button", { name: /UPDATE invoices SET paid_at/ })
    )
    await expect(args.onInspectHistory).toHaveBeenCalledWith(history[1])
    await expect(args.onOpenHistory).not.toHaveBeenCalled()
  },
}

/** A saved query is read the same way, by its title. */
export const SelectReadsASavedQuery: Story = {
  args: { view: "saved", state: { status: "saved", entries: saved } },
  play: async ({ canvas, args }) => {
    await userEvent.click(
      canvas.getByRole("button", { name: "unpaid invoices.sql" })
    )
    await expect(args.onInspectSaved).toHaveBeenCalledWith(saved[0])
    await expect(args.onOpenSaved).not.toHaveBeenCalled()
  },
}

/**
 * A copy names the connection it opens on, which is not the one the entry
 * lists: the history row ran on another connection than this workspace's.
 */
export const OpenCopyNamesTheDestination: Story = {
  args: {
    currentConnectionName: "billing staging",
    state: {
      status: "history",
      entries: [{ ...history[0]!, connectionName: "billing primary" }],
    },
  },
  play: async ({ canvas, args }) => {
    await userEvent.click(
      canvas.getByRole("button", { name: "Open copy in billing staging" })
    )
    await expect(args.onOpenHistory).toHaveBeenCalledTimes(1)
  },
}

/** Saved queries name the destination the same way. */
export const SavedOpenCopyNamesTheDestination: Story = {
  args: { view: "saved", state: { status: "saved", entries: saved } },
  play: async ({ canvas }) => {
    await expect(
      canvas.getAllByRole("button", { name: "Open copy in billing replica" })
    ).toHaveLength(2)
  },
}

/**
 * Marking a write reconciled names its connection and quotes the statement
 * first; the default button keeps the warning, so Enter acknowledges nothing.
 */
export const MarkReconciled: Story = {
  play: async ({ canvas, args }) => {
    const marks = canvas.getAllByRole("button", { name: /Mark reconciled/ })
    await expect(marks).toHaveLength(1)
    await userEvent.click(marks[0]!)
    const dialog = await screen.findByRole("alertdialog")
    await expect(dialog).toHaveTextContent("billing primary")
    await expect(dialog).toHaveTextContent(
      "UPDATE invoices SET paid_at = now() WHERE id = $1"
    )
    await expect(dialog).not.toHaveTextContent(/018f0000/)
    const keep = await screen.findByRole("button", { name: "Keep warning" })
    await waitFor(() => expect(keep).toHaveFocus())
    await userEvent.keyboard("{Enter}")
    await expect(args.onReconcile).not.toHaveBeenCalled()

    await userEvent.click(
      canvas.getByRole("button", { name: /Mark reconciled/ })
    )
    await userEvent.click(
      await screen.findByRole("button", { name: "Mark reconciled" })
    )
    await expect(args.onReconcile).toHaveBeenCalledWith(history[1])
  },
}

/** A reconciled write says so, and no longer asks for inspection. */
export const Reconciled: Story = {
  args: {
    state: {
      status: "history",
      entries: [{ ...history[1]!, needsInspection: false, reconciled: true }],
    },
  },
  play: async ({ canvas }) => {
    await expect(canvas.getByText("Reconciled")).toBeVisible()
    await expect(canvas.queryByText("Needs inspection")).toBeNull()
    await expect(
      canvas.queryByRole("button", { name: /Mark reconciled/ })
    ).toBeNull()
  },
}

/** Only a run of this connection offers its rows; another's stays a copy. */
export const OpenRetainedResult: Story = {
  play: async ({ canvas, args }) => {
    const open = canvas.getAllByRole("button", { name: /Open result/ })
    await expect(open).toHaveLength(1)
    await userEvent.click(open[0]!)
    await expect(args.onOpenResult).toHaveBeenCalledWith(history[0])
  },
}

/** Hostile and right-to-left text stays text, truncated in place. */
export const HostileText: Story = {
  args: {
    state: {
      status: "history",
      entries: [
        {
          ...history[0]!,
          connectionName: "مخزن الفواتير — <img src=x onerror=alert(1)>",
          preview:
            'SELECT * FROM "users"; DROP TABLE audit; -- \u202Egnp.exe 🧾 '.repeat(
              8
            ),
        },
      ],
    },
  },
  play: async ({ canvas }) => {
    await expect(canvas.getByText(/<img src=x/)).toBeVisible()
  },
}

export const SavedQueries: Story = {
  args: { view: "saved", state: { status: "saved", entries: saved } },
  play: async ({ canvas, args }) => {
    // Only a query of this connection can be resumed.
    await expect(
      canvas.getAllByRole("button", { name: /Resume/ })
    ).toHaveLength(1)
    await userEvent.click(
      canvas.getByRole("button", { name: "Delete unpaid invoices.sql" })
    )
    const cancel = await screen.findByRole("button", { name: "Cancel" })
    await waitFor(() => expect(cancel).toHaveFocus())
    await userEvent.keyboard("{Enter}")
    await expect(args.onDeleteSaved).not.toHaveBeenCalled()
  },
}

/**
 * A long connection name and statement with nothing to break on: the
 * reconciliation dialog keeps them inside its frame.
 */
export const ReconcileLongContentStaysInTheFrame: Story = {
  args: {
    state: {
      status: "history",
      entries: [
        {
          ...history[1]!,
          connectionName:
            "analytics_warehouse_production_eu_west_3_read_replica",
          preview: `UPDATE reporting_warehouse_2026.customer_orders_with_shipping_details SET ${"shipping_address_line_two_".repeat(8)}= NULL`,
        },
      ],
    },
  },
  play: async ({ canvas }) => {
    await userEvent.click(
      canvas.getByRole("button", { name: /Mark reconciled/ })
    )
    await expectContainedInFrame(await openFrame("alert-dialog-content"))
  },
}

/** The same for deleting a saved query whose title has nothing to break on. */
export const DeleteLongTitleStaysInTheFrame: Story = {
  args: {
    view: "saved",
    state: {
      status: "saved",
      entries: [
        {
          ...saved[0]!,
          title:
            "reporting_warehouse_2026_customer_orders_with_shipping_details.sql",
        },
      ],
    },
  },
  play: async ({ canvas }) => {
    await userEvent.click(canvas.getByRole("button", { name: /^Delete / }))
    await expectContainedInFrame(await openFrame("alert-dialog-content"))
  },
}

export const Loading: Story = {
  args: { state: { status: "loading" } },
}

export const Empty: Story = {
  args: { state: { status: "history", entries: [] }, hasNext: false },
}

/** A malformed request fails the same way again: no retry, the next step. */
export const Failed: Story = {
  args: {
    state: {
      status: "error",
      error: {
        message:
          "local list limit must be 1..=200 and search at most 1024 bytes",
        retryable: false,
      },
    },
  },
  play: async ({ canvas, args }) => {
    await expect(canvas.getByText(/local list limit/)).toBeVisible()
    await expect(
      canvas.getByText("Change the search or the filters, then refresh.")
    ).toBeVisible()
    await expect(canvas.queryByRole("button", { name: "Try again" })).toBeNull()
    await expect(args.onRefresh).not.toHaveBeenCalled()
  },
}

/** A transient failure says so, and offers the retry as the user's click. */
export const FailedRetryable: Story = {
  args: {
    state: {
      status: "error",
      error: { message: "database is locked", retryable: true },
    },
  },
  play: async ({ canvas, args }) => {
    await expect(canvas.getByText(/This error is transient/)).toBeVisible()
    await userEvent.click(canvas.getByRole("button", { name: "Try again" }))
    await expect(args.onRefresh).toHaveBeenCalledTimes(1)
  },
}

/** A read in flight can be stopped; the refresh comes back afterwards. */
export const Searching: Story = {
  args: { state: { status: "loading" }, searching: true },
  play: async ({ canvas, args }) => {
    await expect(canvas.queryByRole("button", { name: /Refresh/ })).toBeNull()
    await userEvent.click(canvas.getByRole("button", { name: "Cancel search" }))
    await expect(args.onCancel).toHaveBeenCalledTimes(1)
  },
}

/** A cancelled search is not an empty result: it claims no match. */
export const Cancelled: Story = {
  args: { state: { status: "cancelled" }, hasNext: false },
  play: async ({ canvas }) => {
    await expect(canvas.getByText("Search cancelled")).toBeVisible()
    await expect(
      canvas.queryByText("No queries match these filters.")
    ).toBeNull()
    await expect(canvas.getByRole("button", { name: /Refresh/ })).toBeEnabled()
  },
}
