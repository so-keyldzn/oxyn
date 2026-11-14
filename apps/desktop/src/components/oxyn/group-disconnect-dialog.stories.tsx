import type { Meta, StoryObj } from "@storybook/react-vite"
import { fn, expect, screen, within, waitFor } from "storybook/test"
import { GroupDisconnectDialog } from "./group-disconnect-dialog"

const meta = {
  title: "Oxyn/GroupDisconnectDialog",
  component: GroupDisconnectDialog,
  args: {
    name: "Development",
    members: [
      {
        id: "example",
        name: "Example database",
        consoles: 2,
        unsaved: 1,
        running: 0,
        transaction: null,
      },
    ],
    onReview: fn(),
    onConfirm: fn(),
    onCancel: fn(),
  },
} satisfies Meta<typeof GroupDisconnectDialog>
export default meta
type Story = StoryObj<typeof meta>
export const Drafts: Story = {
  play: async ({ userEvent, args }) => {
    const canvas = within(await screen.findByRole("dialog"))
    await waitFor(() =>
      expect(canvas.getByRole("button", { name: "Cancel" })).toHaveFocus()
    )
    await userEvent.keyboard("{Enter}")
    await expect(args.onCancel).toHaveBeenCalled()
    await expect(args.onConfirm).not.toHaveBeenCalled()
  },
}
export const Empty: Story = { args: { members: [] } }
export const Running: Story = {
  args: {
    members: [
      {
        id: "example",
        name: "Example database",
        consoles: 1,
        unsaved: 0,
        running: 1,
        transaction: null,
      },
    ],
  },
}
export const UnknownTransaction: Story = {
  args: {
    members: [
      {
        id: "example",
        name: "Example database",
        consoles: 1,
        unsaved: 0,
        running: 0,
        transaction: "Transaction state unknown",
      },
    ],
  },
  play: async () => {
    const canvas = within(await screen.findByRole("dialog"))
    await expect(
      canvas.getByRole("button", { name: "Continue to draft reviews" })
    ).toBeDisabled()
  },
}
