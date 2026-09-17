import type { Meta, StoryObj } from "@storybook/react-vite"
import { expect, fn, userEvent } from "storybook/test"

import { invoicesDetail } from "./fixtures"
import { invoiceConstraints, invoiceIndexes } from "./metadata-fixtures"
import { RelationConstraints } from "./relation-constraints"

// Loading, empty, stale and error states are the frame's: see `FacetFrame`.
const meta = {
  title: "Oxyn/RelationConstraints",
  component: RelationConstraints,
  decorators: [
    (Story) => (
      <div className="h-[420px] w-[960px] overflow-auto">
        <Story />
      </div>
    ),
  ],
  args: {
    constraints: invoiceConstraints,
    detail: invoicesDetail,
    indexes: invoiceIndexes,
    onCopyDefinition: fn(),
  },
} satisfies Meta<typeof RelationConstraints>

export default meta
type Story = StoryObj<typeof meta>

export const Populated: Story = {
  play: async ({ canvas, args }) => {
    await expect(canvas.getByText("Not validated")).toBeVisible()
    await expect(canvas.getByText("NOT NULL columns")).toBeVisible()
    await userEvent.click(
      canvas.getByRole("button", {
        name: "Copy the definition of invoices_amount_positive",
      })
    )
    await expect(args.onCopyDefinition).toHaveBeenCalledWith(
      "CHECK (amount > 0)"
    )
  },
}

/** Summaries are omitted, not « none », when the relation was not read. */
export const WithoutTheRelationRead: Story = {
  args: { detail: null, indexes: null },
  play: async ({ canvas }) => {
    await expect(canvas.queryByText("NOT NULL columns")).toBeNull()
    await expect(canvas.queryByText(/None/)).toBeNull()
  },
}
