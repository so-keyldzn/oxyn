import * as React from "react"
import type { Meta, StoryObj } from "@storybook/react-vite"
import { expect, userEvent, waitFor } from "storybook/test"

import { PrivacyTierBadge, PrivacyTierField } from "./privacy-tier"
import type { PrivacyTier } from "@/lib/ipc/types"

function Controlled({ initial }: { initial: PrivacyTier }) {
  const [tier, setTier] = React.useState(initial)
  return <PrivacyTierField value={tier} onChange={setTier} />
}

const meta = {
  title: "Oxyn/PrivacyTier",
  component: Controlled,
  decorators: [
    (Story) => (
      <div className="max-w-xl p-6">
        <Story />
      </div>
    ),
  ],
  args: { initial: "metadata" },
} satisfies Meta<typeof Controlled>

export default meta
type Story = StoryObj<typeof meta>

export const MetadataIsNotNothing: Story = {
  play: async ({ canvas }) => {
    // ADR-0006: the default sends the structure, and says so.
    await expect(
      canvas.getByText(/Metadata is not « nothing leaves »/)
    ).toBeVisible()
    await expect(
      canvas.getByRole("list", { name: "Sent to the provider" })
    ).toHaveTextContent("Query plans")
    await userEvent.click(canvas.getByRole("radio", { name: /^Sampled/ }))
    await waitFor(() =>
      expect(
        canvas.getByRole("list", { name: "Sent to the provider" })
      ).toHaveTextContent("Row samples you approve")
    )
  },
}

export const Local: Story = { args: { initial: "local" } }

export const Badges: Story = {
  render: () => (
    <div className="flex gap-2">
      <PrivacyTierBadge tier="local" />
      <PrivacyTierBadge tier="metadata" />
      <PrivacyTierBadge tier="sampled" />
    </div>
  ),
}
