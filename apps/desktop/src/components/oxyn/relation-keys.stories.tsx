import type { Meta, StoryObj } from "@storybook/react-vite"
import { expect, fn, userEvent } from "storybook/test"

import {
  HOSTILE,
  customerKey,
  hostileAddress,
  incomingKeys,
} from "./metadata-fixtures"
import { IncomingKeys, OutgoingKeys } from "./relation-keys"

// Loading, empty, stale and error states are the frame's: see `FacetFrame`.
const meta = {
  title: "Oxyn/RelationKeys",
  component: IncomingKeys,
  decorators: [
    (Story) => (
      <div className="h-[320px] w-[960px] overflow-auto">
        <Story />
      </div>
    ),
  ],
  args: {
    keys: incomingKeys,
    onOpen: fn(),
    onReviewRelatedRows: fn(),
  },
} satisfies Meta<typeof IncomingKeys>

export default meta
type Story = StoryObj<typeof meta>

/** A hostile source name is text, and opening it passes its segments. */
export const Incoming: Story = {
  play: async ({ canvas, args }) => {
    await expect(
      canvas.getByText(`billing.public.${HOSTILE} (last_invoice)`)
    ).toBeVisible()
    await expect(canvas.getByText("Many to one")).toBeVisible()
    // Each action names its own row: a hostile table name reaches the label
    // as text, and two rows are never the same name twice.
    const open = canvas.getAllByRole("button", { name: /^Open source table / })
    await expect(open[1]).toHaveAccessibleName(expect.stringContaining(HOSTILE))
    await userEvent.click(open[1]!)
    await expect(args.onOpen).toHaveBeenCalledWith(hostileAddress)
    const review = canvas.getAllByRole("button", {
      name: "Review related-row query",
    })
    await userEvent.click(review[0]!)
    await expect(args.onReviewRelatedRows).toHaveBeenCalledWith(0)
  },
}

/** Without a console to review in, no action is offered rather than a dead one. */
export const IncomingReadOnly: Story = {
  args: { onOpen: undefined, onReviewRelatedRows: undefined },
  play: async ({ canvas }) => {
    await expect(canvas.queryByRole("button")).toBeNull()
  },
}

export const Outgoing: Story = {
  render: (args) => (
    <OutgoingKeys
      keys={[customerKey]}
      onOpen={args.onOpen}
      onReviewRelatedRows={args.onReviewRelatedRows}
    />
  ),
  play: async ({ canvas, args }) => {
    await expect(canvas.getByText("cascade")).toBeVisible()
    // The name carries the row's target: one row per key, and « Open
    // referenced table » alone would be the same name ten times over.
    const open = canvas.getByRole("button", {
      name: /^Open referenced table /,
    })
    await expect(open).toHaveAccessibleName(
      expect.stringContaining("customers")
    )
    await userEvent.click(open)
    await expect(args.onOpen).toHaveBeenCalledWith(customerKey.references)
  },
}
