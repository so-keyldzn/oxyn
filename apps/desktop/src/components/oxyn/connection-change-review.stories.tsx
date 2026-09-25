import type { Meta, StoryObj } from "@storybook/react-vite"
import { expect, fn, userEvent, waitFor, within } from "storybook/test"

import { ConnectionChangeReview } from "./connection-change-review"
import { expectContainedInFrame, openFrame } from "./frame-overflow"

const meta = {
  title: "Oxyn/ConnectionChangeReview",
  component: ConnectionChangeReview,
  args: {
    onDecide: fn(),
    change: {
      command: "018f0000-0000-7000-8000-00000000c0de",
      kind: "update",
      reason: 'DDL operation on "billing", marked production',
      connectionName: "billing",
      environment: "production",
    },
  },
} satisfies Meta<typeof ConnectionChangeReview>

export default meta
type Story = StoryObj<typeof meta>

export const ProductionEdit: Story = {
  play: async ({ args }) => {
    const dialog = within(document.body)
    const cancel = await dialog.findByRole("button", { name: "Cancel" })
    // The dialog animates in; under a loaded test run that takes a while.
    await waitFor(() => expect(cancel).toHaveFocus(), { timeout: 3000 })
    await waitFor(() =>
      expect(dialog.getByRole("button", { name: "Save billing" })).toBeVisible()
    )
    // Enter away from the buttons decides nothing; Escape rejects.
    cancel.blur()
    await userEvent.keyboard("{Enter}")
    await expect(args.onDecide).not.toHaveBeenCalled()
    await userEvent.keyboard("{Escape}")
    await waitFor(() => expect(args.onDecide).toHaveBeenCalledWith(false))
  },
}

export const ProductionDeletion: Story = {
  args: {
    change: {
      command: "018f0000-0000-7000-8000-00000000c0df",
      kind: "delete",
      reason: 'DDL operation on "billing", marked production',
      connectionName: "billing",
      environment: "production",
    },
  },
  play: async ({ args }) => {
    const dialog = within(document.body)
    const remove = await dialog.findByRole("button", { name: "Delete billing" })
    await userEvent.click(remove)
    await expect(args.onDecide).toHaveBeenCalledWith(true)
  },
}

export const Deciding: Story = { args: { deciding: true } }

/** A long name with nothing to break on stays inside the frame. */
export const LongContentStaysInTheFrame: Story = {
  args: {
    change: {
      command: "018f0000-0000-7000-8000-00000000c0e2",
      kind: "delete",
      reason:
        'DDL operation on "analytics_warehouse_production_eu_west_3_read_replica", marked production',
      connectionName: "analytics_warehouse_production_eu_west_3_read_replica",
      environment: "production",
    },
  },
  play: async () => {
    await expectContainedInFrame(await openFrame("alert-dialog-content"))
  },
}

/** The same, in a compact window. */
export const LongContentStaysInTheFrameWhenCompact: Story = {
  ...LongContentStaysInTheFrame,
  globals: { viewport: { value: "mobile1" } },
}
