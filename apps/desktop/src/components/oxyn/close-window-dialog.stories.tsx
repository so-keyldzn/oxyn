import type { Meta, StoryObj } from "@storybook/react-vite"
import { expect, fn, screen, userEvent, waitFor } from "storybook/test"

import { CloseWindowDialog } from "./close-window-dialog"
import type { CloseWindowRow } from "./close-window-dialog"

const REPORT: CloseWindowRow = {
  key: "console:1",
  title: "monthly report.sql",
  connection: "billing",
  unsaved: true,
  conflict: false,
  running: false,
}

const IMPORT: CloseWindowRow = {
  key: "console:2",
  title: "console_2.sql",
  connection: "analytics",
  unsaved: false,
  conflict: false,
  running: true,
}

const meta = {
  title: "Oxyn/CloseWindowDialog",
  component: CloseWindowDialog,
  args: {
    rows: [REPORT],
    connections: ["billing"],
    onCancel: fn(),
    onClose: fn(),
  },
} satisfies Meta<typeof CloseWindowDialog>

export default meta
type Story = StoryObj<typeof meta>

/**
 * One console would lose its text: the window is named by its connection,
 * the console and what it would lose are listed, and the reflex answer —
 * Enter on the focused `Cancel` — keeps the window and the text.
 */
export const OneConsoleWouldLoseWork: Story = {
  play: async ({ args }) => {
    const dialog = await screen.findByRole("alertdialog")
    await expect(dialog).toHaveTextContent("Close the window of billing?")
    await expect(dialog).toHaveTextContent("monthly report.sql")
    await expect(dialog).toHaveTextContent("Unsaved SQL")
    await expect(dialog).not.toHaveTextContent("console:1")
    const cancel = await screen.findByRole("button", { name: "Cancel" })
    await waitFor(() => expect(cancel).toHaveFocus())
    await userEvent.keyboard("{Enter}")
    await expect(args.onClose).not.toHaveBeenCalled()
    await expect(args.onCancel).toHaveBeenCalled()
  },
}

/**
 * Several consoles across the window's workspaces, one of them running: one
 * dialog for all of them, and closing is an explicit click.
 */
export const SeveralConsolesInOneDialog: Story = {
  args: { rows: [REPORT, IMPORT], connections: ["billing", "analytics"] },
  play: async ({ args }) => {
    const dialog = await screen.findByRole("alertdialog")
    await expect(dialog).toHaveTextContent(
      "Close the window of billing, analytics?"
    )
    const list = await screen.findByRole("list", {
      name: "Consoles that would lose work",
    })
    await expect(list.querySelectorAll("li")).toHaveLength(2)
    await expect(dialog).toHaveTextContent("Operation running")
    await userEvent.click(
      await screen.findByRole("button", { name: "Discard and close window" })
    )
    await expect(args.onClose).toHaveBeenCalledOnce()
    await expect(args.onCancel).not.toHaveBeenCalled()
  },
}

/** The consoles are closing: the close cannot be sent twice. */
export const Closing: Story = {
  args: { busy: true },
  play: async () => {
    // The spinner names itself too: the button's name only contains the words.
    await expect(
      await screen.findByRole("button", { name: /Discard and close window/ })
    ).toBeDisabled()
    await expect(
      await screen.findByRole("button", { name: "Cancel" })
    ).toBeEnabled()
  },
}

/** A window with no workspace is named as such. */
export const WindowWithoutConnection: Story = {
  args: { connections: [] },
  play: async () => {
    await expect(await screen.findByRole("alertdialog")).toHaveTextContent(
      "Close this window?"
    )
  },
}

export const Closed: Story = {
  args: { rows: null },
  play: async () => {
    await expect(screen.queryByRole("alertdialog")).toBeNull()
  },
}
