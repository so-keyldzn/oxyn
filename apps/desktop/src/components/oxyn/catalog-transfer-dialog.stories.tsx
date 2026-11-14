import type { Meta, StoryObj } from "@storybook/react-vite"
import { fn, expect, screen, within, waitFor } from "storybook/test"
import { CatalogTransferDialog } from "./catalog-transfer-dialog"

const meta = {
  title: "Oxyn/CatalogTransferDialog",
  component: CatalogTransferDialog,
  args: {
    name: "public.orders",
    connection: "Example development",
    onSource: fn(),
    onCancel: fn(),
  },
} satisfies Meta<typeof CatalogTransferDialog>
export default meta
type Story = StoryObj<typeof meta>
export const ConfirmSource: Story = {
  play: async ({ userEvent, args }) => {
    const canvas = within(await screen.findByRole("dialog"))
    const cancel = canvas.getByRole("button", { name: "Cancel" })
    await waitFor(() => expect(cancel).toHaveFocus())
    await userEvent.keyboard("{Enter}")
    await expect(args.onCancel).toHaveBeenCalled()
    await expect(args.onSource).not.toHaveBeenCalled()
  },
}
