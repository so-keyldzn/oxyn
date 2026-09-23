import type { Meta, StoryObj } from "@storybook/react-vite"
import { expect, userEvent, waitFor, within } from "storybook/test"

import { StatusBar } from "./status-bar"

const postgres = [
  "SERVER_SIDE_CANCEL",
  "EXPLAIN",
  "EXPLAIN_ANALYZE",
  "AFFECTED_ROWS",
  "INDEXES",
  "CONSTRAINTS",
  "FOREIGN_KEYS",
]

const meta = {
  title: "Oxyn/StatusBar",
  component: StatusBar,
  args: {
    connectionName: "billing",
    driver: "postgres",
    environment: "production",
    readOnly: false,
    execution: { status: "idle" },
    capabilities: postgres,
  },
} satisfies Meta<typeof StatusBar>

export default meta
type Story = StoryObj<typeof meta>

export const Idle: Story = {
  play: async ({ canvas }) => {
    // Without a declared provider, no privacy badge (UX-SPEC).
    await expect(canvas.queryByText(/AI · /)).toBeNull()
  },
}
export const Running: Story = {
  args: { execution: { status: "running", rows: 184_000 } },
}
export const Done: Story = {
  args: { execution: { status: "done", rows: 1_000_000, elapsedMs: 2_341 } },
}
export const ReadOnlyReplica: Story = {
  args: {
    connectionName: "billing replica",
    environment: "staging",
    readOnly: true,
  },
}
export const ReadOnlySession: Story = {
  args: {
    environment: "staging",
    capabilities: [...postgres, "READ_ONLY_SESSION"],
  },
  play: async ({ canvas }) => {
    await expect(canvas.getByText("Read only")).toBeVisible()
  },
}
export const Failed: Story = { args: { execution: { status: "failed" } } }

export const FailedWithServerMessage: Story = {
  args: {
    execution: {
      status: "failed",
      message:
        'ERROR:  duplicate key value violates unique constraint "invoices_pkey" (SQLSTATE 23505)',
      retryable: false,
    },
  },
  play: async ({ canvas }) => {
    await expect(canvas.getByRole("button", { name: "Details" })).toBeVisible()
  },
}

export const Cancelling: Story = {
  args: { execution: { status: "cancelling", rows: 184_000 } },
  play: async ({ canvas }) => {
    await expect(canvas.getAllByText(/Cancellation requested/)[0]).toBeVisible()
  },
}

export const Cancelled: Story = { args: { execution: { status: "cancelled" } } }

export const RowCountIsNotReadAloud: Story = {
  args: { execution: { status: "running", rows: 1_234_567 } },
  play: async ({ canvasElement }) => {
    // The live region says the state; the counter stays visual.
    const live = canvasElement.querySelector("[aria-live]")
    await expect(live).toHaveTextContent("Running")
    await expect(live).not.toHaveTextContent("1,234,567")
  },
}

export const Narrow: Story = {
  decorators: [
    (Story) => (
      <div className="w-[420px]">
        <Story />
      </div>
    ),
  ],
  args: {
    connectionName: "production-eu-west-analytics-warehouse-primary",
    readOnly: true,
    execution: { status: "running", rows: 18_000 },
  },
  play: async ({ canvasElement }) => {
    const bar = canvasElement.querySelector<HTMLElement>(
      "[data-slot=status-bar]"
    )
    // Nothing overflows: the name truncates, secondary facts fold away.
    await expect(bar!.scrollWidth).toBeLessThanOrEqual(bar!.clientWidth)
  },
}

export const HostileName: Story = {
  args: { connectionName: "قاعدة \u202Etxt.exe <b>billing</b>" },
}

export const Light: Story = {
  args: { readOnly: true },
  globals: { theme: "light" },
}
export const SessionCapabilities: Story = {
  play: async ({ canvas }) => {
    await userEvent.click(
      canvas.getByRole("button", { name: "Session capabilities" })
    )
    const popover = within(document.body)
    // An absent capability is said, not hidden (ADR-0003).
    await waitFor(() =>
      expect(
        popover.getByText(
          "This session has no transaction: rollback is offered only when supported."
        )
      ).toBeVisible()
    )
  },
}
