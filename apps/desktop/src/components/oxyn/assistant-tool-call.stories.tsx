import type { Meta, StoryObj } from "@storybook/react-vite"
import { expect, fn, userEvent, within } from "storybook/test"

import { AssistantRestoredCall, AssistantToolCall } from "./assistant-tool-call"
import { APPROVAL_ID } from "./assistant-fixtures"
import type {
  RestoredCallEntry,
  ToolCallEntry,
} from "@/features/assistant/transcript"

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
  result: null,
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

const restored: RestoredCallEntry = {
  kind: "restoredCall",
  key: "restored-1",
  tool: "execute_query",
  summary: "7 rows, 1 batches",
  statement:
    "WITH category_sales AS (\n  SELECT c.name AS category,\n         SUM(oi.quantity * oi.unit_price * (1 - oi.discount)) AS revenue\n  FROM main.order_items AS oi\n  JOIN main.products AS p ON p.id = oi.product_id\n  JOIN main.categories AS c ON c.id = p.category_id\n  GROUP BY c.name\n)\nSELECT category, revenue\nFROM category_sales\nORDER BY revenue DESC;",
  status: "completed",
  errorClass: null,
  rowsNotKept: true,
}

/**
 * A call of a conversation reopened after a restart: the card it had live,
 * without what the workspace does not keep — its connection, whether it could
 * write, its rows.
 */
export const Restored: Story = {
  render: () => <AssistantRestoredCall entry={restored} />,
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement)
    await expect(
      canvas.getByRole("region", { name: "Command execute_query: Completed" })
    ).toBeVisible()
    const statement = canvas.getByLabelText("Statement the agent submitted")
    await expect(statement.textContent).toBe(restored.statement)
    // Wrapped at spaces, never inside a word.
    await expect(getComputedStyle(statement).whiteSpace).toBe("pre-wrap")
    await expect(getComputedStyle(statement).wordBreak).not.toBe("break-all")
    await expect(canvas.getByText("7 rows, 1 batches")).toBeVisible()
    await expect(
      canvas.getByText(/Result no longer available: the workspace keeps/)
    ).toBeVisible()
    // Not recorded, so not claimed.
    await expect(canvas.queryByText("Read only")).toBeNull()
    await expect(canvas.queryByText("May change data")).toBeNull()
  },
}

export const RestoredFailed: Story = {
  render: () => (
    <AssistantRestoredCall
      entry={{
        ...restored,
        summary: 'relation "main.missing" does not exist',
        statement: "SELECT * FROM main.missing;",
        status: "failed",
        errorClass: "permanent",
        rowsNotKept: false,
      }}
    />
  ),
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement)
    await expect(
      canvas.getByRole("region", { name: "Command execute_query: Failed" })
    ).toBeVisible()
    await expect(canvas.getByText(/permanent —/)).toBeVisible()
    await expect(canvas.queryByText(/Result no longer available/)).toBeNull()
  },
}
