import type { Meta, StoryObj } from "@storybook/react-vite"
import { expect, fn, userEvent, waitFor, within } from "storybook/test"

import { ApprovalDialog } from "./approval-dialog"
import { expectContainedInFrame, openFrame } from "./frame-overflow"

const meta = {
  title: "Oxyn/ApprovalDialog",
  component: ApprovalDialog,
  args: {
    connectionName: "billing",
    environment: "production",
    onDecide: fn(),
    approval: {
      command: "018f0000-0000-7000-8000-00000000c0de",
      reason: "Writes on a production connection need a review.",
      preview: {
        statement:
          "DELETE FROM invoices\nWHERE issued_at < now() - interval '7 years'\n  AND paid_at IS NOT NULL;",
        connection: "billing",
        estimatedRows: 48_213,
      },
    },
  },
} satisfies Meta<typeof ApprovalDialog>

export default meta
type Story = StoryObj<typeof meta>

export const ProductionWrite: Story = {
  play: async ({ args }) => {
    const dialog = within(document.body)
    const cancel = await dialog.findByRole("button", { name: "Cancel" })
    // Cancel takes the initial focus; the destructive action names the connection.
    await waitFor(() => expect(cancel).toHaveFocus())
    // The dialog animates in: wait for the action rather than read the frame
    // where it is still transparent.
    await waitFor(() =>
      expect(
        dialog.getByRole("button", { name: "Run on billing" })
      ).toBeVisible()
    )

    // Enter pressed away from the buttons does nothing at all: it neither
    // approves nor rejects.
    dialog.getByLabelText("Statement to approve").focus()
    await userEvent.keyboard("{Enter}")
    await expect(args.onDecide).not.toHaveBeenCalled()

    // Escape rejects.
    await userEvent.keyboard("{Escape}")
    await waitFor(() => expect(args.onDecide).toHaveBeenCalledWith(false))
  },
}

export const Deciding: Story = {
  args: { deciding: true },
}

export const AgentRequest: Story = {
  args: {
    actor: { kind: "agent", name: "SQL assistant" },
    environment: "staging",
  },
  play: async () => {
    const dialog = within(document.body)
    // The dialog animates in: wait for it rather than read its first frame.
    const [line] = await dialog.findAllByText("An agent (SQL assistant)")
    await waitFor(() => expect(line).toBeVisible())
  },
}

export const LocalUnknownRows: Story = {
  args: {
    environment: "local",
    approval: {
      command: "018f0000-0000-7000-8000-00000000c0e0",
      reason: "Schema changes need a review.",
      preview: {
        statement: "DROP TABLE scratch;",
        connection: "scratch.sqlite",
        estimatedRows: null,
      },
    },
  },
  play: async () => {
    const dialog = within(document.body)
    const [line] = await dialog.findAllByText(
      "The number of affected rows is unknown."
    )
    await waitFor(() => expect(line).toBeVisible())
  },
}

/**
 * A connection name is whatever the user typed. It is bounded inside the
 * action — the dialogue still names it in full above — so a 200-character
 * name cannot push the buttons out of the window.
 */
export const HostileConnectionName: Story = {
  args: {
    connectionName: `prod "; DROP TABLE audit; -- ${"long".repeat(40)}`,
    approval: {
      command: "018f0000-0000-7000-8000-00000000c0e0",
      reason: "Writes on a production connection need a review.",
      preview: {
        statement: "DELETE FROM invoices WHERE paid_at IS NULL;",
        connection: `prod "; DROP TABLE audit; -- ${"long".repeat(40)}`,
        estimatedRows: 3,
      },
    },
  },
  play: async () => {
    const dialog = within(document.body)
    const action = await dialog.findByRole("button", { name: /^Run on prod/ })
    // The action stays inside the footer it sits in, and the name it cannot
    // show in full is still reachable.
    await waitFor(() => {
      const room = action.parentElement?.getBoundingClientRect().width ?? 0
      expect(action.getBoundingClientRect().width).toBeLessThanOrEqual(room + 1)
    })
    await expect(action).toHaveAttribute(
      "title",
      expect.stringContaining("Run on prod")
    )
  },
}

/**
 * Long names everywhere — the agent, the connection, a statement line with no
 * space to break on: the header, the statement and the footer stay inside the
 * dialog's frame, at the window's default width.
 */
export const LongContentStaysInTheFrame: Story = {
  args: {
    actor: {
      kind: "agent",
      name: "Codex — the agent of the analytics team's staging workspace",
    },
    connectionName: "analytics_warehouse_production_eu_west_3_read_replica",
    approval: {
      command: "018f0000-0000-7000-8000-00000000c0e1",
      reason: "Writes on a production connection need a review.",
      preview: {
        statement: `UPDATE reporting_warehouse_2026.customer_orders_with_shipping_details SET ${"shipping_address_line_two_".repeat(8)}= NULL;`,
        connection: "analytics_warehouse_production_eu_west_3_read_replica",
        estimatedRows: 12_345_678,
      },
    },
  },
  play: async () => {
    await expectContainedInFrame(await openFrame("alert-dialog-content"))
  },
}

/** The same, in a compact window: the footer stacks, Cancel first. */
export const LongContentStaysInTheFrameWhenCompact: Story = {
  ...LongContentStaysInTheFrame,
  globals: { viewport: { value: "mobile1" } },
}

export const LongStatement: Story = {
  args: {
    approval: {
      command: "018f0000-0000-7000-8000-00000000c0df",
      reason: "DDL on a production connection needs a review.",
      preview: {
        statement: Array.from(
          { length: 60 },
          (_, index) => `ALTER TABLE invoices ADD COLUMN extra_${index} text;`
        ).join("\n"),
        connection: "billing",
        estimatedRows: null,
      },
    },
  },
  play: async () => {
    const dialog = within(document.body)
    const preview = await dialog.findByLabelText("Statement to approve")
    // The whole statement is reviewable from the keyboard.
    preview.focus()
    await userEvent.keyboard("{PageDown}")
    await waitFor(() => expect(preview.scrollTop).toBeGreaterThan(0))
  },
}
