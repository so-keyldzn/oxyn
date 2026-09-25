import type { Meta, StoryObj } from "@storybook/react-vite"
import { expect } from "storybook/test"

import { TransactionBadge } from "./transaction-badge"

const meta = {
  title: "Oxyn/TransactionBadge",
  component: TransactionBadge,
  args: { state: "open" },
} satisfies Meta<typeof TransactionBadge>

export default meta
type Story = StoryObj<typeof meta>

/** Said in words, and announced when it appears: the icon alone says nothing. */
export const Open: Story = {
  play: async ({ canvas }) => {
    const badge = canvas.getByRole("status")
    await expect(badge).toHaveTextContent("Transaction open")
    await expect(badge).toHaveAttribute("data-state", "open")
  },
}

/** Never shown as « no transaction »: the session did not say. */
export const Unknown: Story = {
  args: { state: "unknown" },
  play: async ({ canvas }) => {
    await expect(canvas.getByRole("status")).toHaveTextContent(
      "Transaction state unknown"
    )
  },
}

/** In the dark theme the badge stays legible: axe checks its contrast. */
export const Dark: Story = {
  decorators: [
    (Story) => (
      <div className="dark bg-background p-3 text-foreground">
        <Story />
      </div>
    ),
  ],
  play: async ({ canvas }) => {
    await expect(canvas.getByText("Transaction open")).toBeVisible()
  },
}
