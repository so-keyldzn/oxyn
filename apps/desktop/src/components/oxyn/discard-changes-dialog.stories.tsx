import type { Meta, StoryObj } from "@storybook/react-vite"
import { expect, fn, userEvent, waitFor, within } from "storybook/test"

import { DiscardChangesDialog } from "./discard-changes-dialog"

const meta = {
  title: "Oxyn/DiscardChangesDialog",
  component: DiscardChangesDialog,
  args: {
    open: true,
    what: "this new connection",
    onKeep: fn(),
    onDiscard: fn(),
  },
} satisfies Meta<typeof DiscardChangesDialog>

export default meta
type Story = StoryObj<typeof meta>

export const KeepIsTheDefault: Story = {
  play: async ({ args }) => {
    const body = within(document.body)
    const keep = await body.findByRole("button", { name: "Keep editing" })
    await waitFor(() => expect(keep).toHaveFocus())
    // A reflex Enter keeps the typed values.
    await userEvent.keyboard("{Enter}")
    await expect(args.onKeep).toHaveBeenCalled()
    await expect(args.onDiscard).not.toHaveBeenCalled()
  },
}

export const EscapeKeeps: Story = {
  play: async ({ args }) => {
    const body = within(document.body)
    await body.findByRole("alertdialog")
    await userEvent.keyboard("{Escape}")
    await expect(args.onKeep).toHaveBeenCalled()
    await expect(args.onDiscard).not.toHaveBeenCalled()
  },
}

export const Discard: Story = {
  play: async ({ args }) => {
    const body = within(document.body)
    await userEvent.click(await body.findByRole("button", { name: "Discard" }))
    await expect(args.onDiscard).toHaveBeenCalledOnce()
  },
}

export const Light: Story = { globals: { theme: "light" } }
