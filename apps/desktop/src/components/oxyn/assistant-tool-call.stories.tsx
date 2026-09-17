import type { Meta, StoryObj } from "@storybook/react-vite"
import { expect, fn, userEvent, within } from "storybook/test"

import { AssistantToolCall } from "./assistant-tool-call"
import { APPROVAL_ID } from "./assistant-fixtures"
import type { ToolCallEntry } from "@/features/assistant/transcript"

const base: ToolCallEntry = {
  kind: "tool",
  key: "tool-1-0",
  call: 0,
  tool: "execute_query",
  command: "Execute",
  statement: "SELECT status, count(*)\nFROM public.orders\nGROUP BY status;",
  connection: "commerce-prod",
  environment: "production",
  mutating: false,
  state: "running",
  detail: null,
  errorClass: null,
  withheld: false,
  rows: null,
  approval: null,
}

const write: ToolCallEntry = {
  ...base,
  statement: "UPDATE invoices SET paid_at = now()\nWHERE id = 4211;",
  connection: "billing-staging",
  environment: "staging",
  mutating: true,
}

const meta = {
  title: "Oxyn/Assistant/ToolCall",
  component: AssistantToolCall,
  args: { entry: base, onReview: fn() },
  decorators: [
    (Story) => (
      <div className="max-w-xl p-4">
        <Story />
      </div>
    ),
  ],
} satisfies Meta<typeof AssistantToolCall>

export default meta
type Story = StoryObj<typeof meta>

export const Running: Story = {}

export const Completed: Story = {
  args: {
    entry: { ...base, state: "completed", detail: "3 rows, 1 batches" },
  },
}

export const AwaitingApproval: Story = {
  args: {
    entry: {
      ...write,
      state: "awaitingApproval",
      detail: "Writes on this connection need a review.",
      approval: {
        id: APPROVAL_ID,
        reason: "Writes on this connection need a review.",
        statement: write.statement ?? "",
        connection: "billing-staging",
        environment: "staging",
      },
    },
  },
  play: async ({ canvasElement, args }) => {
    const canvas = within(canvasElement)
    await expect(canvas.getByText("May change data")).toBeVisible()
    await expect(canvas.getAllByText("billing-staging").length).toBeGreaterThan(
      0
    )
    // The pending command's id is data, never text on screen.
    await expect(canvasElement.textContent).not.toContain(APPROVAL_ID)
    await userEvent.click(canvas.getByRole("button", { name: "Review…" }))
    await expect(args.onReview).toHaveBeenCalledOnce()
  },
}

export const Deciding: Story = {
  args: {
    entry: {
      ...write,
      state: "deciding",
      approval: {
        id: APPROVAL_ID,
        reason: "r",
        statement: write.statement ?? "",
        connection: "billing-staging",
        environment: "staging",
      },
    },
  },
}

export const DeniedOnProduction: Story = {
  args: {
    entry: {
      ...write,
      connection: "commerce-prod",
      environment: "production",
      state: "denied",
      detail: "agents may not write on a production connection",
    },
  },
}

export const RejectedByUser: Story = {
  args: {
    entry: {
      ...write,
      state: "rejected",
      detail: "You rejected this statement. Nothing ran.",
    },
  },
}

export const FailedAmbiguous: Story = {
  args: {
    entry: {
      ...write,
      state: "failed",
      errorClass: "ambiguous",
      detail: "canceling statement due to statement timeout (SQLSTATE 57014)",
    },
  },
  play: async ({ canvasElement }) => {
    // I-13: an ambiguous failure says it may have applied.
    await expect(
      within(canvasElement).getByText(/it may have applied/)
    ).toBeVisible()
  },
}

export const Cancelled: Story = {
  args: { entry: { ...base, state: "cancelled", detail: "cancelled" } },
}

export const Withheld: Story = {
  args: {
    entry: {
      ...write,
      state: "failed",
      errorClass: "permanent",
      withheld: true,
      detail:
        'duplicate key value violates unique constraint "clients_email_key": Key (email)=(dupont@example.com) already exists.',
    },
  },
}
