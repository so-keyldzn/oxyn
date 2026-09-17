import type { Meta, StoryObj } from "@storybook/react-vite"
import { expect } from "storybook/test"

import { invoiceIndexes } from "./metadata-fixtures"
import { RelationIndexes } from "./relation-indexes"

// Loading, empty, stale and error states are the frame's: see `FacetFrame`.
const meta = {
  title: "Oxyn/RelationIndexes",
  component: RelationIndexes,
  decorators: [
    (Story) => (
      <div className="h-[320px] w-[900px] overflow-auto">
        <Story />
      </div>
    ),
  ],
  args: { indexes: invoiceIndexes },
} satisfies Meta<typeof RelationIndexes>

export default meta
type Story = StoryObj<typeof meta>

export const Populated: Story = {
  play: async ({ canvas }) => {
    await expect(canvas.getByText("Unique")).toBeVisible()
    await expect(canvas.getByText("paid_at IS NULL")).toBeVisible()
  },
}

export const Unreported: Story = {
  args: {
    indexes: [
      {
        name: "",
        fields: ["email"],
        unique: false,
        method: null,
        predicate: null,
      },
    ],
  },
  play: async ({ canvas }) => {
    await expect(canvas.getByText("Unnamed")).toBeVisible()
    await expect(canvas.getByText("Not reported")).toBeVisible()
  },
}
