import type { Meta, StoryObj } from "@storybook/react-vite"
import { fn } from "storybook/test"
import { ServerProfileDialog } from "./server-profile-dialog"

const meta = {
  title: "Oxyn/ServerProfileDialog",
  component: ServerProfileDialog,
  args: {
    name: "Development server",
    removing: false,
    busy: false,
    error: null,
    onSave: fn(),
    onClose: fn(),
  },
} satisfies Meta<typeof ServerProfileDialog>
export default meta
type Story = StoryObj<typeof meta>
export const Create: Story = {}
export const Removing: Story = { args: { removing: true } }
export const Saving: Story = { args: { busy: true } }
export const Error: Story = {
  args: {
    error: {
      message:
        "Example group changed in another view. Refresh before trying again.",
      retryable: false,
    },
  },
}
