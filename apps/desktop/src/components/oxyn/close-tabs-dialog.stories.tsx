import type { Meta, StoryObj } from "@storybook/react-vite"
import { expect, fn, screen, userEvent, waitFor, within } from "storybook/test"

import { CloseTabsDialog } from "./close-tabs-dialog"

const tabs = ["orders.sql", "invoices.sql"].map((title, index) => ({
  key: String(index),
  title,
  blocked: null,
  reasons: {
    title,
    connection: "billing",
    unsaved: true,
    conflict: false,
    running: index === 0,
    transaction: index === 0 ? ("open" as const) : null,
    hasSavedCopy: true,
  },
}))
const meta = {
  title: "Oxyn/CloseTabsDialog",
  component: CloseTabsDialog,
  args: {
    review: { tabs, busy: false, notice: null },
    onCancel: fn(),
    onSave: fn(),
    onDiscard: fn(),
  },
} satisfies Meta<typeof CloseTabsDialog>
export default meta
type Story = StoryObj<typeof meta>

export const CollectiveReview: Story = {
  play: async ({ args }) => {
    const dialog = await screen.findByRole("alertdialog")
    const controls = within(dialog)
    await waitFor(() =>
      expect(controls.getByRole("button", { name: "Cancel" })).toHaveFocus()
    )
    await expect(dialog).toHaveTextContent("orders.sql")
    await expect(dialog).toHaveTextContent("invoices.sql")
    await expect(dialog).toHaveTextContent("A transaction is open on billing")
    controls.getByRole("list", { name: "Tabs to close" }).focus()
    await userEvent.keyboard("{Enter}")
    await expect(args.onDiscard).not.toHaveBeenCalled()
    await expect(args.onSave).not.toHaveBeenCalled()
    await userEvent.click(
      controls.getByRole("button", { name: "Discard all and close" })
    )
    await expect(args.onDiscard).toHaveBeenCalledOnce()
  },
}
export const Busy: Story = {
  args: { review: { tabs, busy: true, notice: null } },
  play: async ({ args }) => {
    const dialog = within(await screen.findByRole("alertdialog"))
    await expect(
      dialog.getByRole("button", { name: "Save all and close" })
    ).toBeDisabled()
    await expect(
      dialog.getByRole("button", { name: "Discard all and close" })
    ).toBeDisabled()
    await userEvent.click(dialog.getByRole("button", { name: "Cancel" }))
    await expect(args.onCancel).toHaveBeenCalledOnce()
  },
}
export const FailedSave: Story = {
  args: {
    review: {
      tabs,
      busy: false,
      notice: "Not saved: invoices.sql. No tabs were closed.",
    },
  },
  play: async () => {
    await expect(await screen.findByRole("alert")).toHaveTextContent(
      "No tabs were closed"
    )
  },
}
export const BlockedExport: Story = {
  args: {
    review: {
      tabs: [
        {
          key: "result",
          title: "Query result",
          reasons: null,
          blocked: "Finish or cancel the export before closing this tab.",
        },
      ],
      busy: false,
      notice: null,
    },
  },
  play: async () => {
    const dialog = within(await screen.findByRole("alertdialog"))
    await expect(
      dialog.getByRole("button", { name: "Discard all and close" })
    ).toBeDisabled()
  },
}
