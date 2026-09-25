import type { Meta, StoryObj } from "@storybook/react-vite"
import { expect, fn, userEvent, waitFor, within } from "storybook/test"

import { DeleteConnectionDialog } from "./delete-connection-dialog"
import { summaries } from "./connection-fixtures"
import { expectContainedInFrame, openFrame } from "./frame-overflow"

const billing = summaries[0]!

const meta = {
  title: "Oxyn/DeleteConnectionDialog",
  component: DeleteConnectionDialog,
  args: { connection: billing, onCancel: fn(), onConfirm: fn() },
} satisfies Meta<typeof DeleteConnectionDialog>

export default meta
type Story = StoryObj<typeof meta>

export const NamesTheConnection: Story = {
  play: async ({ args }) => {
    const dialog = within(document.body)
    const cancel = await dialog.findByRole("button", { name: "Cancel" })
    await waitFor(() => expect(cancel).toHaveFocus())

    // The action names the connection and stays locked until it is typed.
    const remove = dialog.getByRole("button", { name: "Delete billing" })
    await expect(remove).toBeDisabled()

    const input = dialog.getByLabelText("Type the connection name to confirm")
    await userEvent.type(input, "billin")
    await expect(remove).toBeDisabled()
    // Enter in the field deletes nothing, even once the name matches.
    await userEvent.type(input, "g{Enter}")
    await expect(args.onConfirm).not.toHaveBeenCalled()
    await expect(remove).toBeEnabled()

    await userEvent.click(remove)
    await expect(args.onConfirm).toHaveBeenCalledWith(billing)
  },
}

export const EscapeCancels: Story = {
  play: async ({ args }) => {
    const dialog = within(document.body)
    const cancel = await dialog.findByRole("button", { name: "Cancel" })
    await waitFor(() => expect(cancel).toHaveFocus())
    await userEvent.keyboard("{Escape}")
    await waitFor(() => expect(args.onCancel).toHaveBeenCalled())
    await expect(args.onConfirm).not.toHaveBeenCalled()
  },
}

export const Deleting: Story = {
  args: { deleting: true },
}

export const Refused: Story = {
  args: {
    error: "Operation rejected",
  },
}

export const Closed: Story = {
  args: { connection: null },
}

export const HomonymShowsItsLocation: Story = {
  args: {
    connection: {
      ...billing,
      location: "db-eu.internal:5432 / billing",
    },
  },
  play: async () => {
    const dialog = within(document.body)
    const location = await dialog.findByText("db-eu.internal:5432 / billing")
    await waitFor(() => expect(location).toBeVisible())
  },
}

export const HostileRightToLeftName: Story = {
  args: {
    connection: {
      ...billing,
      name: 'قاعدة "users"; DROP TABLE audit; --‮txt.exe',
    },
  },
  play: async ({ args }) => {
    const dialog = within(document.body)
    const input = await dialog.findByLabelText(
      "Type the connection name to confirm"
    )
    // A near miss never unlocks the action.
    await userEvent.type(input, "قاعدة")
    await expect(dialog.getByRole("button", { name: /^Delete/ })).toBeDisabled()
    await expect(args.onConfirm).not.toHaveBeenCalled()
  },
}

/**
 * A long name and location with nothing to break on, and a long refusal: the
 * header, the field and the footer stay inside the frame.
 */
export const LongContentStaysInTheFrame: Story = {
  args: {
    connection: {
      ...billing,
      name: "analytics_warehouse_production_eu_west_3_read_replica",
      location:
        "analytics-warehouse-production-eu-west-3.cluster-ro.rds.amazonaws.com:5432/reporting",
    },
    error:
      "keyring: the entry oxyn/analytics-warehouse-production-eu-west-3-read-replica/password could not be removed",
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

export const Light: Story = { globals: { theme: "light" } }
