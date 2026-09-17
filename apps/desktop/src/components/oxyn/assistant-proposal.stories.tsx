import type { Meta, StoryObj } from "@storybook/react-vite"
import { expect, fn, userEvent, within } from "storybook/test"

import { AssistantProposal } from "./assistant-proposal"

const sql = [
  '-- Proposed change · "public"."orders" · commerce-prod',
  "-- Nothing has been executed. Uncomment one statement, complete it, and review it",
  "-- before running: this connection is named above for that reason.",
  "--",
  "-- Rename the column. Replace new_name before running.",
  '-- ALTER TABLE "public"."orders" RENAME COLUMN "status" TO new_name;',
  "--",
  "-- Change nullability. The column is currently nullable.",
  '-- ALTER TABLE "public"."orders" ALTER COLUMN "status" SET NOT NULL;',
  "",
].join("\n")

const meta = {
  title: "Oxyn/Metadata/ProposedChange",
  component: AssistantProposal,
  args: {
    relation: "public.orders",
    connectionName: "commerce-prod",
    environment: "production",
    state: {
      status: "ready",
      sql,
      title: "Proposed change.sql",
      origin:
        "a schema-change template · uncomment and complete one statement before running",
    },
    onOpenInConsole: fn(),
  },
  decorators: [
    (Story) => (
      <div className="max-w-2xl p-4">
        <Story />
      </div>
    ),
  ],
} satisfies Meta<typeof AssistantProposal>

export default meta
type Story = StoryObj<typeof meta>

export const Ready: Story = {
  play: async ({ canvasElement, args }) => {
    const canvas = within(canvasElement)
    await userEvent.click(
      canvas.getByRole("button", { name: "Open in console" })
    )
    await expect(args.onOpenInConsole).toHaveBeenCalledWith(
      sql,
      "Proposed change.sql"
    )
    // Every line is commented: nothing runs on a distracted Run (ADR-0025).
    const text = String(args.onOpenInConsole.mock.calls[0]?.[0])
    for (const line of text.split("\n").filter((part) => part !== "")) {
      await expect(line.startsWith("--")).toBe(true)
    }
  },
}

export const Composing: Story = { args: { state: { status: "loading" } } }

export const NothingToPropose: Story = {
  args: { environment: "local", state: { status: "unavailable" } },
}

export const Failed: Story = {
  args: {
    state: { status: "error", message: "This connection has no catalog cache" },
  },
}
