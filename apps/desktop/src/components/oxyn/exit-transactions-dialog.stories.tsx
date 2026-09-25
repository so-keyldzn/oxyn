import type { Meta, StoryObj } from "@storybook/react-vite"
import { expect, fn, screen, userEvent, waitFor } from "storybook/test"

import { ExitTransactionsDialog } from "./exit-transactions-dialog"
import type { ExitTransactionRow } from "./exit-transactions-dialog"

/** Session ids are opaque and never shown; this one is synthetic. */
const INVOICES: ExitTransactionRow = {
  session: "session-a",
  console: "unpaid invoices.sql",
  connectionName: "billing",
  environment: "production",
  state: "open",
}

const DRAFT: ExitTransactionRow = {
  session: "session-b",
  console: "console_2.sql",
  connectionName: "analytics copy",
  environment: "staging",
  state: "unknown",
}

const meta = {
  title: "Oxyn/ExitTransactionsDialog",
  component: ExitTransactionsDialog,
  args: {
    rows: [INVOICES],
    onCommit: fn(),
    onRollback: fn(),
    onCancel: fn(),
  },
} satisfies Meta<typeof ExitTransactionsDialog>

export default meta
type Story = StoryObj<typeof meta>

/**
 * One transaction on production: the console, the connection and its
 * environment are named, and the reflex answer — Enter on the focused
 * `Cancel` — keeps Oxyn open without committing or rolling back.
 */
export const OneOpenTransaction: Story = {
  play: async ({ args }) => {
    const dialog = await screen.findByRole("alertdialog")
    await expect(dialog).toHaveTextContent("Quit with an open transaction?")
    await expect(dialog).toHaveTextContent("unpaid invoices.sql")
    await expect(dialog).toHaveTextContent("billing")
    await expect(dialog).toHaveTextContent("PRODUCTION")
    await expect(dialog).toHaveTextContent("Transaction open")
    await expect(dialog).not.toHaveTextContent("session-a")
    const cancel = await screen.findByRole("button", { name: "Cancel" })
    await waitFor(() => expect(cancel).toHaveFocus())
    await userEvent.keyboard("{Enter}")
    await expect(args.onCommit).not.toHaveBeenCalled()
    await expect(args.onRollback).not.toHaveBeenCalled()
    await expect(args.onCancel).toHaveBeenCalled()
  },
}

/** Several consoles, one whose state could not be read: never « idle ». */
/**
 * The close of a window that is not the last (ADR-0043): the same decision,
 * on that window's consoles alone, and it speaks of the window, not of
 * quitting.
 */
export const ClosingOneWindow: Story = {
  args: { scope: "window" },
  play: async ({ args }) => {
    const dialog = await screen.findByRole("alertdialog")
    await expect(dialog).toHaveTextContent(
      "Close this window with an open transaction?"
    )
    await expect(dialog).not.toHaveTextContent("Quit")
    const cancel = await screen.findByRole("button", { name: "Cancel" })
    await waitFor(() => expect(cancel).toHaveFocus())
    await userEvent.keyboard("{Enter}")
    await expect(args.onCommit).not.toHaveBeenCalled()
    await expect(args.onCancel).toHaveBeenCalled()
  },
}

export const SeveralWithUnknownState: Story = {
  args: { rows: [INVOICES, DRAFT] },
  play: async ({ args }) => {
    const dialog = await screen.findByRole("alertdialog")
    await expect(dialog).toHaveTextContent("Quit with open transactions?")
    await expect(dialog).toHaveTextContent("Transaction state unknown")
    await expect(dialog).toHaveTextContent("Staging")
    await userEvent.click(await screen.findByRole("button", { name: "Commit" }))
    await expect(args.onCommit).toHaveBeenCalledOnce()
  },
}

export const RollbackChosen: Story = {
  play: async ({ args }) => {
    await userEvent.click(
      await screen.findByRole("button", { name: "Rollback" })
    )
    await expect(args.onRollback).toHaveBeenCalledOnce()
    await expect(args.onCommit).not.toHaveBeenCalled()
  },
}

/** While `COMMIT` runs, the decisions are off and `Cancel` stays live. */
export const Committing: Story = {
  args: { busy: "commit" },
  play: async ({ args }) => {
    // The spinner lends its own label to the button it sits in.
    await expect(
      await screen.findByRole("button", { name: /Commit$/ })
    ).toBeDisabled()
    await expect(
      await screen.findByRole("button", { name: "Rollback" })
    ).toBeDisabled()
    const cancel = await screen.findByRole("button", { name: "Cancel" })
    await expect(cancel).toBeEnabled()
    await userEvent.click(cancel)
    await expect(args.onCancel).toHaveBeenCalled()
  },
}

/**
 * The server refused the `COMMIT`: its message shows as it came, the dialog
 * stays open, and `COMMIT` is not offered again (I-13) — `Rollback` and
 * `Cancel` remain.
 */
export const CommitRefused: Story = {
  args: {
    error: "FOREIGN KEY constraint failed",
    commitBlocked: true,
  },
  play: async () => {
    await expect(await screen.findByRole("alert")).toHaveTextContent(
      "FOREIGN KEY constraint failed"
    )
    await expect(
      await screen.findByRole("button", { name: "Commit" })
    ).toBeDisabled()
    await expect(
      await screen.findByRole("button", { name: "Rollback" })
    ).toBeEnabled()
  },
}

/**
 * Every session came back idle after a failure: nothing is left to decide,
 * and the exit does not go on by itself.
 */
export const NothingLeftAfterAFailure: Story = {
  args: {
    rows: [],
    error: "COMMIT was cancelled; its outcome is not known.",
  },
  play: async () => {
    const dialog = await screen.findByRole("alertdialog")
    await expect(dialog).toHaveTextContent("No transaction is open any more.")
    await expect(
      await screen.findByRole("button", { name: "Commit" })
    ).toBeDisabled()
    await expect(
      await screen.findByRole("button", { name: "Rollback" })
    ).toBeDisabled()
  },
}

export const Closed: Story = {
  args: { rows: null },
  play: async () => {
    await expect(screen.queryByRole("alertdialog")).toBeNull()
  },
}
