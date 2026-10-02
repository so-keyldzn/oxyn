import type { Meta, StoryObj } from "@storybook/react-vite"
import { expect, fn, screen, userEvent, waitFor } from "storybook/test"

import { RestartToUpdateDialog } from "./restart-to-update-dialog"

const meta = {
  title: "Oxyn/RestartToUpdateDialog",
  component: RestartToUpdateDialog,
  args: {
    work: { running: 1, exports: 0 },
    onLater: fn(),
    onStopAndRestart: fn(),
  },
} satisfies Meta<typeof RestartToUpdateDialog>

export default meta
type Story = StoryObj<typeof meta>

/** A query runs on a server: restarting cancels it there, and says so. */
export const RunningQueries: Story = {
  args: { work: { running: 2, exports: 0 } },
  play: async () => {
    const dialog = await screen.findByRole("alertdialog")
    await expect(dialog).toHaveTextContent("Restart now and stop running work?")
    await expect(dialog).toHaveTextContent(
      "2 queries are running. Restarting cancels them on the server."
    )
    await expect(dialog).toHaveTextContent(
      "Your consoles come back in their windows, offline."
    )
    await expect(dialog).not.toHaveTextContent("export")
  },
}

/** An export stops; the destination keeps what it held before. */
export const ExportInProgress: Story = {
  args: { work: { running: 0, exports: 1 } },
  play: async () => {
    const dialog = await screen.findByRole("alertdialog")
    await expect(dialog).toHaveTextContent(
      "1 export is in progress. It stops, and the destination file keeps its previous content."
    )
    await expect(dialog).not.toHaveTextContent("query")
  },
}

/**
 * Both at once. The reflex answer is `Later`: it has the focus, and Enter on
 * the dialog's body stops nothing.
 */
export const Both: Story = {
  args: { work: { running: 1, exports: 2 } },
  play: async ({ args }) => {
    const dialog = await screen.findByRole("alertdialog")
    await expect(dialog).toHaveTextContent("1 query is running.")
    await expect(dialog).toHaveTextContent("2 exports are in progress.")
    const later = await screen.findByRole("button", { name: "Later" })
    await waitFor(() => expect(later).toHaveFocus())
    // Enter from the body, not from a button, is inert.
    const description = dialog.querySelector(
      "[data-slot=alert-dialog-description]"
    )
    if (!(description instanceof HTMLElement)) throw new Error("no body")
    description.tabIndex = -1
    description.focus()
    await userEvent.keyboard("{Enter}")
    await expect(args.onStopAndRestart).not.toHaveBeenCalled()
    // Still open: the opening animation may not have ended under load.
    await waitFor(() => expect(screen.getByRole("alertdialog")).toBeVisible())
    await userEvent.click(
      await screen.findByRole("button", { name: "Stop and restart" })
    )
    await expect(args.onStopAndRestart).toHaveBeenCalledOnce()
  },
}

export const Closed: Story = {
  args: { work: null },
  play: async () => {
    await expect(screen.queryByRole("alertdialog")).toBeNull()
  },
}
