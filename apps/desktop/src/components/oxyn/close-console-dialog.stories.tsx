import type { Meta, StoryObj } from "@storybook/react-vite"
import { expect, fn, screen, userEvent, waitFor } from "storybook/test"

import { CloseConsoleDialog } from "./close-console-dialog"
import { expectContainedInFrame, openFrame } from "./frame-overflow"

const meta = {
  title: "Oxyn/CloseConsoleDialog",
  component: CloseConsoleDialog,
  args: {
    reasons: {
      title: "unpaid invoices.sql",
      connection: "billing",
      conflict: false,
      hasSavedCopy: true,
      unsaved: true,
      running: false,
      transaction: null,
    },
    onCancel: fn(),
    onSave: fn(),
    onDiscard: fn(),
  },
} satisfies Meta<typeof CloseConsoleDialog>

export default meta
type Story = StoryObj<typeof meta>

/** Unsaved work asks, and the reflex answer keeps it. */
export const UnsavedWork: Story = {
  play: async ({ args }) => {
    const dialog = await screen.findByRole("alertdialog")
    const cancel = await screen.findByRole("button", { name: "Cancel" })
    await waitFor(() => expect(cancel).toHaveFocus())
    await userEvent.keyboard("{Enter}")
    await expect(args.onDiscard).not.toHaveBeenCalled()
    await expect(args.onCancel).toHaveBeenCalled()
    await expect(dialog).toHaveTextContent("Closing discards this text.")
  },
}

export const RunningStatement: Story = {
  args: {
    reasons: {
      title: "console_2.sql",
      connection: "billing",
      conflict: false,
      hasSavedCopy: false,
      unsaved: false,
      running: true,
      transaction: null,
    },
  },
}

export const Conflict: Story = {
  args: {
    reasons: {
      title: "monthly.sql",
      connection: "billing",
      conflict: true,
      hasSavedCopy: true,
      unsaved: true,
      running: false,
      transaction: null,
    },
    notice:
      "The stored query changed elsewhere. Save a new query to preserve both versions.",
  },
  play: async ({ args }) => {
    await userEvent.click(
      await screen.findByRole("button", { name: "Save copy and close" })
    )
    await expect(args.onSave).toHaveBeenCalled()
  },
}

/** While saving, Cancel stays live: it cancels the save under way. */
export const Saving: Story = {
  args: { busy: true },
  play: async ({ args }) => {
    await expect(
      await screen.findByRole("button", { name: "Discard and close" })
    ).toBeDisabled()
    const cancel = await screen.findByRole("button", { name: "Cancel" })
    await expect(cancel).toBeEnabled()
    await userEvent.click(cancel)
    await expect(args.onCancel).toHaveBeenCalled()
  },
}

/**
 * A transaction the session reported open: closing rolls it back, so the
 * dialog asks even though the text is saved, names the connection, says the
 * transaction is rolled back — and the reflex answer keeps the console
 * (ADR-0039 §5).
 */
export const OpenTransaction: Story = {
  args: {
    reasons: {
      title: "console_3.sql",
      connection: "billing",
      conflict: false,
      hasSavedCopy: true,
      unsaved: false,
      running: false,
      transaction: "open",
    },
  },
  play: async ({ args }) => {
    const dialog = await screen.findByRole("alertdialog")
    await expect(dialog).toHaveTextContent("A transaction is open on billing.")
    await expect(dialog).toHaveTextContent("rolls it back")
    const cancel = await screen.findByRole("button", { name: "Cancel" })
    await waitFor(() => expect(cancel).toHaveFocus())
    await userEvent.keyboard("{Enter}")
    await expect(args.onDiscard).not.toHaveBeenCalled()
    await expect(args.onCancel).toHaveBeenCalled()
  },
}

/** The session could not say: the dialog does not say « no transaction ». */
export const UnknownTransactionState: Story = {
  args: {
    reasons: {
      title: "console_4.sql",
      connection: "billing",
      conflict: false,
      hasSavedCopy: true,
      unsaved: false,
      running: false,
      transaction: "unknown",
    },
  },
  play: async () => {
    const dialog = await screen.findByRole("alertdialog")
    await expect(dialog).toHaveTextContent(
      "The transaction state on billing is unknown."
    )
    await waitFor(() =>
      expect(screen.getByRole("button", { name: "Cancel" })).toHaveFocus()
    )
  },
}

/** A long file name with nothing to break on stays inside the frame. */
export const LongContentStaysInTheFrame: Story = {
  args: {
    reasons: {
      title:
        "reporting_warehouse_2026_customer_orders_with_shipping_details.sql",
      connection: "analytics-warehouse-production-eu-west-1-read-replica",
      conflict: true,
      hasSavedCopy: true,
      unsaved: true,
      running: true,
      transaction: "open",
    },
    notice:
      "Not saved: /Users/analyst/Documents/workspaces/analytics-warehouse-production/consoles/reporting_warehouse_2026.sql is read-only.",
  },
  play: async () => {
    await expectContainedInFrame(await openFrame("alert-dialog-content"))
  },
}

/** The same, in a compact window: the three actions stack. */
export const LongContentStaysInTheFrameWhenCompact: Story = {
  ...LongContentStaysInTheFrame,
  globals: { viewport: { value: "mobile1" } },
}
