import type { Meta, StoryObj } from "@storybook/react-vite"
import { expect, fn, userEvent, waitFor, within } from "storybook/test"

import { visibleAssistantField } from "./assistant-composer"
import { AssistantToolRows } from "./assistant-tool-rows"
import { invoiceColumns, syntheticPages } from "./fixtures"
import { AssistantView } from "./assistant-view"
import type { PlanEntry } from "./assistant-plan"
import type { AssistantSource } from "./assistant-sources"
import type { ContextPin } from "./assistant-context-pins"
import {
  APPROVAL_ID,
  agentStarted,
  answer,
  answered,
  assistantState,
  destinations,
  enabledEntry,
  remoteProvider,
  started,
  threadOfEvents,
} from "./assistant-fixtures"
import { NO_USABLE_DESTINATION } from "@/features/assistant/availability"
import { NEW_THREAD } from "@/features/assistant/thread"
import type { AiEvent } from "@/lib/ipc/ai"

const question = "How many active clients?"

const answeredThread = threadOfEvents([
  {
    question,
    events: [
      { kind: "question", text: question, mentions: [] },
      started,
      { kind: "turnStarted", turn: 1, maxTurns: 8 },
      {
        kind: "toolCall",
        call: 0,
        tool: "execute_query",
        command: "Execute",
        statement:
          "SELECT count(*) FROM public.clients WHERE deleted_at IS NULL",
        connection: "commerce-prod",
        environment: "production",
        mutating: false,
      },
      {
        kind: "toolReported",
        call: 0,
        status: "completed",
        detail: "1 rows, 1 batches",
        errorClass: null,
        withheld: false,
        rows: 1,
        result: "result-active-clients",
      },
      { kind: "textDelta", text: answer },
      {
        kind: "usage",
        input: 8200,
        output: 410,
        cacheRead: 6000,
        cacheWrite: null,
      },
      answered,
    ],
  },
])

const PLAN: Array<PlanEntry> = [
  {
    content: "Read the structure of clients",
    priority: "high",
    status: "completed",
  },
  {
    content: "Count the active ones",
    priority: "medium",
    status: "inProgress",
  },
]

const SOURCES: Array<AssistantSource> = [
  {
    key: "src-1",
    address: { catalog: "commerce", namespace: "public", relation: "clients" },
    facet: "Structure",
    rows: null,
  },
]

const PINS: Array<ContextPin> = [
  { key: "pin-1", kind: "object", label: "public.clients", shape: "9 columns" },
]

/** The same thread, with the three surfaces the backend does not fill yet. */
const threadWithPlanAndSources = {
  ...answeredThread,
  nodes: answeredThread.nodes.map((node) => ({
    ...node,
    exchange: { ...node.exchange, plan: PLAN, sources: SOURCES },
  })),
}

const meta = {
  title: "Oxyn/Assistant/Panel",
  component: AssistantView,
  args: {
    connectionName: "commerce-prod",
    environment: "production",
    tier: "metadata",
    entry: enabledEntry,
    state: assistantState(NEW_THREAD),
    pins: [],
    selected: destinations[0] ?? null,
    model: remoteProvider.model,
    models: null,
    modelCost: null,
    onSelectDestination: fn(),
    onSelectModel: fn(),
    onModelsWanted: fn(),
    onAsk: fn(() => true),
    onStop: fn(),
    onDecide: fn(),
    onOpenInConsole: fn(),
    onOpenObject: fn(),
    onRemovePin: fn(),
    onRegenerate: fn(),
    onEdit: fn(() => true),
    onContinue: fn(),
    onSelectVersion: fn(),
    onSignIn: fn(),
    onCopy: fn(() => true),
    onNewConversation: fn(),
    onOpenThread: fn(),
    onRenameThread: fn(async () => {}),
    onDeleteThread: fn(async () => {}),
    onReloadHistory: fn(),
    onSendQueued: fn(),
    onRemoveQueued: fn(),
  },
  decorators: [
    (Story) => (
      <div className="h-[640px] max-w-2xl border">
        <Story />
      </div>
    ),
  ],
} satisfies Meta<typeof AssistantView>

export default meta
type Story = StoryObj<typeof meta>

export const Initial: Story = {
  play: async ({ canvasElement, args }) => {
    const canvas = within(canvasElement)
    // A provider reads the catalog under the tier: said as such.
    await expect(
      canvas.getByText(/reads this connection's catalog under its privacy tier/)
    ).toBeVisible()
    // An example fills the field and sends nothing.
    await userEvent.click(
      canvas.getByRole("button", {
        name: "What are the main tables here, and how are they related?",
      })
    )
    const field = canvas.getByRole("textbox", {
      name: "Question for the assistant",
    })
    await expect(field).toHaveTextContent(
      "What are the main tables here, and how are they related?"
    )
    await expect(field).toHaveFocus()
    // What `Ask AI` focuses once the column is drawn.
    await expect(visibleAssistantField(canvasElement)).toBe(field)
    await expect(args.onAsk).not.toHaveBeenCalled()
  },
}

export const InitialWithAnAgent: Story = {
  args: { selected: destinations[1] ?? null },
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement)
    // An agent assembles its own prompt: the provider's sentence would lie.
    await expect(
      canvas.queryByText(/reads this connection's catalog/)
    ).toBeNull()
    await expect(
      canvas.getByText(/receives your question as you type it/)
    ).toBeVisible()
  },
}

export const SendButtonLooksInactiveWhenEmpty: Story = {
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement)
    const send = canvas.getByRole("button", { name: "Send question" })
    await expect(send).toHaveAttribute("aria-disabled", "true")
    // Not the primary colour of a button that sends.
    await expect(send.className).not.toMatch(/\bbg-primary\b/)
    await userEvent.type(
      canvas.getByRole("textbox", { name: "Question for the assistant" }),
      "How many?"
    )
    await expect(send).not.toHaveAttribute("aria-disabled")
    await expect(send.className).toMatch(/\bbg-primary\b/)
  },
}

export const Running: Story = {
  args: {
    state: assistantState(
      threadOfEvents(
        [
          {
            question,
            events: [
              { kind: "question", text: question, mentions: [] },
              started,
              { kind: "turnStarted", turn: 1, maxTurns: 8 },
            ],
          },
        ],
        { running: true }
      )
    ),
  },
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement)
    await expect(canvas.getByText("Waiting for the model…")).toBeVisible()
    // The panel says it once; the waiting marker is not a second region.
    await expect(
      canvas.getByText("Waiting for the model…").closest("[role=status]")
    ).toBeNull()
    await expect(
      canvasElement.querySelector("[data-slot=assistant-status]")
    ).toHaveTextContent("Answering…")
    // Stopping is offered the whole time, not between two turns.
    await expect(
      canvas.getByRole("button", { name: "Stop the assistant" })
    ).toBeEnabled()
  },
}

export const Answered: Story = {
  args: {
    state: assistantState(answeredThread),
    modelCost: { inputPerMillion: 3, outputPerMillion: 15, currency: "USD" },
  },
  play: async ({ canvasElement, args }) => {
    const canvas = within(canvasElement)
    await expect(canvas.getByText("8.2K in")).toBeVisible()
    await userEvent.click(
      canvas.getByRole("button", { name: "Open in console" })
    )
    // With the provenance that signs it (ADR-0023).
    await expect(args.onOpenInConsole).toHaveBeenCalledWith(
      expect.stringContaining("SELECT count(*)"),
      expect.objectContaining({ model: remoteProvider.model })
    )
    await userEvent.click(
      canvas.getByRole("button", { name: "Regenerate answer" })
    )
    await expect(args.onRegenerate).toHaveBeenCalledOnce()
    await expect(canvas.getByText(/^≈ /)).toBeVisible()
    await expect(canvas.getByRole("status")).toHaveTextContent("Answer ready")
  },
}

const mentionedQuestion = "How many @clients per @clients.country?"

/**
 * A sent question keeps its chips — the composer's own — and a chip opens
 * its object. A table dropped since is dimmed and said, not hidden.
 */
export const QuestionWithMentions: Story = {
  args: {
    state: assistantState(
      threadOfEvents([
        {
          question: mentionedQuestion,
          events: [
            {
              kind: "question",
              text: mentionedQuestion,
              mentions: [
                {
                  kind: "table",
                  label: "clients",
                  mention: {
                    kind: "relation",
                    address: {
                      catalog: null,
                      namespace: "public",
                      relation: "clients",
                    },
                    field: null,
                  },
                  missing: false,
                },
                {
                  kind: "column",
                  label: "clients.country",
                  mention: {
                    kind: "relation",
                    address: {
                      catalog: null,
                      namespace: "public",
                      relation: "clients",
                    },
                    field: "country",
                  },
                  missing: true,
                },
              ],
            },
            started,
            { kind: "textDelta", text: "About 4 200." },
            answered,
          ],
        },
      ])
    ),
  },
  play: async ({ canvasElement, args }) => {
    const canvas = within(canvasElement)
    await userEvent.click(
      canvas.getByRole("button", { name: "Open table clients" })
    )
    await expect(args.onOpenObject).toHaveBeenCalledWith({
      catalog: null,
      namespace: "public",
      relation: "clients",
    })
    await expect(canvas.getByText("· not found")).toBeVisible()
    await expect(
      canvas.queryByRole("button", { name: /clients\.country/ })
    ).toBeNull()
  },
}

export const ThePriceOfAnotherModelIsNotShown: Story = {
  args: {
    state: assistantState(answeredThread),
    // The model selected now is not the one that answered.
    model: "claude-haiku-5",
    modelCost: { inputPerMillion: 1, outputPerMillion: 5, currency: "USD" },
  },
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement)
    await expect(canvas.getByText("8.2K in")).toBeVisible()
    // Today's price for another model would be a wrong figure, not a guess.
    await expect(canvas.queryByText(/^≈ /)).toBeNull()
  },
}

export const ThinkingThenAnswering: Story = {
  args: {
    state: assistantState(
      threadOfEvents([
        {
          question,
          events: [
            { kind: "question", text: question, mentions: [] },
            started,
            {
              kind: "thinkingDelta",
              text: "Look at `clients.deleted_at` first.",
            },
            { kind: "thinkingEnded", elapsedMs: 3100 },
            { kind: "textDelta", text: "There are 1,204 active clients." },
            answered,
          ],
        },
      ])
    ),
  },
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement)
    await expect(canvas.getByText("Thought for 3.1 s")).toBeVisible()
    await expect(canvas.queryByText(/deleted_at` first/)).toBeNull()
  },
}

export const SeveralExchangesWithVersions: Story = {
  render: (args) => {
    const base = threadOfEvents([
      {
        question,
        events: [
          { kind: "question", text: question, mentions: [] },
          started,
          { kind: "textDelta", text: "1,204." },
          answered,
        ],
      },
      {
        question: "And last month?",
        events: [
          { kind: "question", text: "And last month?", mentions: [] },
          started,
          { kind: "textDelta", text: "1,190." },
          answered,
        ],
      },
    ])
    const thread = {
      ...base,
      nodes: [
        ...base.nodes,
        {
          id: 2,
          parent: 0,
          exchange: {
            ...base.nodes[1]!.exchange,
            question: "And the month before?",
          },
        },
      ],
      selections: { ...base.selections, "0": 2 },
    }
    return <AssistantView {...args} state={assistantState(thread)} />
  },
  play: async ({ canvasElement, args }) => {
    const canvas = within(canvasElement)
    await expect(canvas.getByText("2 / 2")).toBeVisible()
    await expect(canvas.getByText("And the month before?")).toBeVisible()
    await userEvent.click(
      canvas.getByRole("button", { name: "Previous version" })
    )
    await expect(args.onSelectVersion).toHaveBeenCalledWith(1)
  },
}

export const Empty: Story = {
  args: {
    state: assistantState(
      threadOfEvents([
        {
          question,
          events: [
            { kind: "question", text: question, mentions: [] },
            started,
            answered,
          ],
        },
      ])
    ),
  },
  play: async ({ canvasElement }) => {
    await expect(
      within(canvasElement).getByText(
        "The model answered nothing. This is not a failure."
      )
    ).toBeVisible()
  },
}

export const ProviderFailed: Story = {
  args: {
    state: assistantState(
      threadOfEvents([
        {
          question,
          events: [
            { kind: "question", text: question, mentions: [] },
            started,
            {
              kind: "failed",
              message: "provider error: 529 overloaded_error: Overloaded",
              category: "provider",
              retryable: true,
              signIn: null,
              foundElsewhere: null,
              exit: null,
            },
          ],
        },
      ])
    ),
  },
  play: async ({ canvasElement, args }) => {
    const canvas = within(canvasElement)
    await userEvent.click(canvas.getByRole("button", { name: "Ask again" }))
    // The model call again, never a tool call replayed on its own (I-13).
    await expect(args.onRegenerate).toHaveBeenCalledOnce()
  },
}

/** A failure after a write ran: the backend withholds « Ask again » (I-13). */
export const FailedAfterAWrite: Story = {
  args: {
    state: assistantState(
      threadOfEvents([
        {
          question,
          events: [
            { kind: "question", text: question, mentions: [] },
            started,
            {
              kind: "failed",
              message:
                "provider error: the stream was cut. A command from this answer may already have been applied: check the data before asking again.",
              category: "provider",
              retryable: false,
              signIn: null,
              foundElsewhere: null,
              exit: null,
            },
          ],
        },
      ])
    ),
  },
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement)
    await expect(
      canvas.getByText(/may already have been applied/)
    ).toBeInTheDocument()
    await expect(canvas.queryByRole("button", { name: "Ask again" })).toBeNull()
  },
}

export const RefusedByTier: Story = {
  args: {
    tier: "local",
    state: assistantState(
      threadOfEvents([
        {
          question,
          events: [
            { kind: "question", text: question, mentions: [] },
            {
              kind: "failed",
              message:
                "This connection is local-only, and this provider's endpoint leaves this machine. Nothing was sent.",
              category: "refused",
              retryable: false,
              signIn: null,
              foundElsewhere: null,
              exit: null,
            },
          ],
        },
      ])
    ),
  },
  play: async ({ canvasElement }) => {
    await expect(
      within(canvasElement).queryByRole("button", { name: "Ask again" })
    ).toBeNull()
  },
}

export const AgentNeedsSignIn: Story = {
  args: {
    selected: destinations[1] ?? null,
    model: null,
    state: assistantState(
      threadOfEvents([
        {
          question,
          events: [
            { kind: "question", text: question, mentions: [] },
            {
              kind: "failed",
              message: "the agent needs you to sign in before it can answer",
              category: "agentSignIn",
              retryable: true,
              signIn: {
                agent: "Claude Code",
                methods: [],
                terminalCommand: "claude auth login",
              },
              foundElsewhere: null,
              exit: null,
            },
          ],
        },
      ])
    ),
  },
  play: async ({ canvasElement }) => {
    await expect(
      within(canvasElement).getByText("claude auth login")
    ).toBeVisible()
  },
}

export const AgentWorkingAndRefused: Story = {
  args: {
    selected: destinations[1] ?? null,
    model: null,
    state: assistantState(
      threadOfEvents([
        {
          question: "Read /etc/hosts and tell me the tables",
          events: [
            {
              kind: "question",
              text: "Read /etc/hosts and tell me the tables",
              mentions: [],
            },
            agentStarted,
            { kind: "agentTool", id: "t1", tool: "read", status: "failed" },
            {
              kind: "permissionRefused",
              action: "read",
              reason:
                "Oxyn does not grant agents access to the file system. Ask about the connected database instead.",
            },
            { kind: "textDelta", text: "I cannot read files here." },
            { kind: "contextWindow", used: 12000, size: 200000, cost: null },
            answered,
          ],
        },
      ])
    ),
  },
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement)
    await expect(canvas.getByText(/Oxyn\s+refused/)).toBeVisible()
    await expect(canvas.getByText(/context 12K \/ 200K/)).toBeVisible()
    // Nothing offers to grant it.
    await expect(canvas.queryByRole("button", { name: /Allow/ })).toBeNull()
  },
}

/**
 * An external agent (Claude Code, Codex) ran `execute_query` through Oxyn's
 * MCP server: its rows are drawn under the call exactly as for a provider. The
 * agent's own tool steps stay apart, reduced to their kind.
 */
export const ExternalAgentQueryWithRows: Story = {
  args: {
    selected: destinations[1] ?? null,
    model: null,
    state: assistantState(
      threadOfEvents([
        {
          question: "Show me the last three invoices",
          events: [
            {
              kind: "question",
              text: "Show me the last three invoices",
              mentions: [],
            },
            agentStarted,
            { kind: "agentTool", id: "t1", tool: "think", status: "completed" },
            {
              kind: "toolCall",
              call: 0,
              tool: "execute_query",
              command: "Execute",
              statement:
                "SELECT * FROM public.invoices ORDER BY issued_at DESC LIMIT 3",
              connection: "commerce-prod",
              environment: "production",
              mutating: false,
            },
            {
              kind: "toolReported",
              call: 0,
              status: "completed",
              detail: "3 rows, 1 batches",
              errorClass: null,
              withheld: false,
              rows: 3,
              result: "result-agent-invoices",
            },
            { kind: "textDelta", text: "Here are the last three invoices." },
            answered,
          ],
        },
      ])
    ),
    renderToolRows: (entry) => (
      <AssistantToolRows
        state={{
          status: "open",
          result: entry.result ?? "",
          columns: invoiceColumns,
          rows: entry.rows ?? 0,
          truncated: false,
        }}
        fetchPage={syntheticPages(3, 0)}
        onOpenAll={fn()}
      />
    ),
  },
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement)
    const call = canvas.getByRole("region", { name: /Command execute_query/ })
    await expect(
      await within(call).findByRole("grid", { name: "Rows the query returned" })
    ).toBeVisible()
    await expect(within(call).getByText("Initech")).toBeVisible()
    await expect(within(call).getByText(/^3 rows$/)).toBeVisible()
    await expect(
      within(call).getByText(/shown to you only; the model got the count/)
    ).toBeVisible()
  },
}

export const TruncatedOffersToContinue: Story = {
  args: {
    state: assistantState(
      threadOfEvents([
        {
          question,
          events: [
            { kind: "question", text: question, mentions: [] },
            started,
            { kind: "textDelta", text: "The slowest queries are" },
            {
              kind: "finished",
              ending: {
                type: "answered",
                turns: 1,
                truncated: true,
                cut: "tokenLimit",
              },
            },
          ],
        },
      ])
    ),
  },
  play: async ({ canvasElement, args }) => {
    const canvas = within(canvasElement)
    await expect(canvas.getByText(/It is not finished\./)).toBeVisible()
    await userEvent.click(canvas.getByRole("button", { name: "Continue" }))
    await expect(args.onContinue).toHaveBeenCalledOnce()
  },
}

export const Unavailable: Story = {
  args: {
    tier: "local",
    entry: {
      status: "disabled",
      reason: NO_USABLE_DESTINATION,
      destinations: destinations.map((option) => ({
        ...option,
        usable: false,
        reason: "local-only",
      })),
    },
    selected: null,
  },
  play: async ({ canvasElement, args }) => {
    const canvas = within(canvasElement)
    await expect(
      canvas.getByText("Not available on this connection")
    ).toBeVisible()
    await expect(canvas.queryByText(/^Ask about/)).toBeNull()
    // The full reason once, a short one under the field, and what to do.
    await expect(canvas.getAllByText(NO_USABLE_DESTINATION)).toHaveLength(1)
    await expect(
      canvas.getByText("Not available on this connection.")
    ).toBeVisible()
    await expect(
      canvas.getByText(
        "Declare a local provider in Settings, or change this connection's privacy tier."
      )
    ).toBeVisible()
    // No example to fill a field that cannot send.
    await expect(
      canvas.queryByRole("list", { name: "Example questions" })
    ).toBeNull()
    const field = canvas.getByRole("textbox", {
      name: "Question for the assistant",
    })
    await userEvent.type(field, "anything{Enter}")
    await expect(args.onAsk).not.toHaveBeenCalled()
  },
}

export const Absent: Story = {
  args: { entry: { status: "absent" } },
  play: async ({ canvasElement }) => {
    // Without a declaration there is no AI workspace at all.
    await expect(canvasElement.querySelector("section")).toBeNull()
    await expect(canvasElement.textContent).toBe("")
  },
}

export const PendingApproval: Story = {
  args: {
    environment: "staging",
    connectionName: "billing-staging",
    state: assistantState(
      threadOfEvents([
        {
          question: "Mark invoice 4211 as paid",
          events: [
            {
              kind: "question",
              text: "Mark invoice 4211 as paid",
              mentions: [],
            },
            started,
            {
              kind: "toolCall",
              call: 0,
              tool: "execute_query",
              command: "Execute",
              statement: "UPDATE invoices SET paid_at = now() WHERE id = 4211",
              connection: "billing-staging",
              environment: "staging",
              mutating: true,
            },
            {
              kind: "approvalRequested",
              call: 0,
              approval: APPROVAL_ID,
              reason: "Writes on this connection need a review.",
              statement: "UPDATE invoices SET paid_at = now() WHERE id = 4211",
              connection: "billing-staging",
              environment: "staging",
              actor: "agent",
            },
            {
              kind: "toolReported",
              call: 0,
              status: "awaitingApproval",
              detail: "Writes on this connection need a review.",
              errorClass: null,
              withheld: false,
              rows: null,
              result: null,
            },
            answered,
          ],
        },
      ])
    ),
  },
  play: async ({ canvasElement, args }) => {
    const canvas = within(canvasElement)
    await expect(canvasElement.textContent).not.toContain(APPROVAL_ID)

    // Not « Answered » while a command waits: the work is not done.
    await expect(canvas.queryByText(/^Answered in/)).toBeNull()
    await expect(canvas.getByText(/^Waiting for your review/)).toBeVisible()
    // One live region for the panel, saying what the user must do.
    const regions = canvas.getAllByRole("status")
    await expect(regions).toHaveLength(1)
    await expect(regions[0]).toHaveTextContent("Needs your review")

    // A message sent meanwhile approves nothing.
    const field = canvas.getByRole("textbox", {
      name: "Question for the assistant",
    })
    await userEvent.type(field, "yes, go ahead{Enter}")
    await expect(args.onAsk).toHaveBeenCalledWith("yes, go ahead", [])
    await expect(args.onDecide).not.toHaveBeenCalled()

    // The review names the connection, focuses Cancel, and Enter does not approve.
    await userEvent.click(canvas.getByRole("button", { name: "Review…" }))
    const dialog = within(document.body)
    const cancel = await dialog.findByRole("button", { name: "Cancel" })
    await waitFor(() => expect(cancel).toHaveFocus())
    // Awaited: the popup fades in, and an opacity still at 0 is not « visible ».
    await waitFor(() =>
      expect(
        dialog.getByRole("button", { name: "Run on billing-staging" })
      ).toBeVisible()
    )
    await userEvent.keyboard("{Enter}")
    await expect(args.onDecide).not.toHaveBeenCalledWith(0, APPROVAL_ID, true)
    await userEvent.keyboard("{Escape}")
    await waitFor(() =>
      expect(args.onDecide).toHaveBeenCalledWith(0, APPROVAL_ID, false)
    )
  },
}

export const QueuedWhileAnswering: Story = {
  args: {
    state: assistantState({
      ...threadOfEvents(
        [
          {
            question,
            events: [
              { kind: "question", text: question, mentions: [] },
              started,
            ],
          },
        ],
        { running: true }
      ),
      queue: [{ key: "queued-1", text: "And by status?" }],
    }),
  },
  play: async ({ canvasElement }) => {
    await expect(
      within(canvasElement).getByText(/sent when the current answer ends/)
    ).toBeVisible()
  },
}

export const TheHistoryOpens: Story = {
  args: {
    state: assistantState(answeredThread, {
      history: {
        status: "ready",
        items: [
          {
            id: "42",
            title: question,
            createdAtMs: Date.UTC(2026, 8, 15, 12, 0),
            updatedAtMs: Date.UTC(2026, 8, 15, 12, 30),
            exchanges: 1,
            running: false,
          },
        ],
      },
    }),
  },
  play: async ({ canvasElement, args }) => {
    const canvas = within(canvasElement)
    await userEvent.click(canvas.getByRole("button", { name: "Conversations" }))
    await expect(
      canvas.getByRole("heading", { name: "Conversations" })
    ).toBeVisible()
    await userEvent.click(
      canvas.getByRole("button", { name: "Start a new conversation" })
    )
    await expect(args.onNewConversation).toHaveBeenCalled()
  },
}

export const SendingClosesTheHistory: Story = {
  args: TheHistoryOpens.args,
  play: async ({ canvasElement, args }) => {
    const canvas = within(canvasElement)
    await userEvent.click(canvas.getByRole("button", { name: "Conversations" }))
    await expect(
      canvas.getByRole("heading", { name: "Conversations" })
    ).toBeVisible()
    await userEvent.type(
      canvas.getByRole("textbox", { name: "Question for the assistant" }),
      "And by region?{Enter}"
    )
    await expect(args.onAsk).toHaveBeenCalledWith("And by region?", [])
    // The question lands in a conversation the user can see.
    await expect(
      canvas.queryByRole("heading", { name: "Conversations" })
    ).toBeNull()
    await expect(
      canvasElement.querySelector('[aria-label="Conversation"]')
    ).toBeVisible()
  },
}

export const TakingTheConversationBack: Story = {
  args: { state: assistantState(NEW_THREAD, { opening: true }) },
  play: async ({ canvasElement }) => {
    await expect(
      within(canvasElement).getByText("Taking the conversation back…")
    ).toBeVisible()
  },
}

export const TheBackendRefusedTheQuestion: Story = {
  args: {
    state: assistantState(NEW_THREAD, {
      askError: "This session is no longer open",
    }),
  },
  play: async ({ canvasElement }) => {
    await expect(within(canvasElement).getByRole("alert")).toHaveTextContent(
      "This session is no longer open"
    )
  },
}

const hostile: AiEvent = {
  kind: "textDelta",
  text: "<script>window.__pwned = true</script>\n\n[go](javascript:window.__pwned=true)",
}

export const HostileAnswerRendersNothingActive: Story = {
  args: {
    state: assistantState(
      threadOfEvents([
        {
          question,
          events: [
            { kind: "question", text: question, mentions: [] },
            started,
            hostile,
            answered,
          ],
        },
      ])
    ),
  },
  play: async ({ canvasElement }) => {
    await expect(canvasElement.querySelector("script")).toBeNull()
    await expect(canvasElement.querySelector("a")).toBeNull()
    await expect(
      (window as unknown as { __pwned?: boolean }).__pwned
    ).toBeUndefined()
  },
}

export const NarrowWindow: Story = {
  args: { state: assistantState(answeredThread) },
  decorators: [
    (Story) => (
      <div className="h-[560px] w-[320px] border">
        <Story />
      </div>
    ),
  ],
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement)
    // The panel is docked to a side: at its narrowest it still shows the
    // answer and the way to ask again, and never scrolls sideways.
    await expect(
      canvas.getByRole("textbox", { name: "Question for the assistant" })
    ).toBeVisible()
    await expect(canvasElement.scrollWidth).toBeLessThanOrEqual(
      canvasElement.clientWidth
    )
  },
}

/**
 * The plan, the sources and the context pins drawn inside the panel.
 *
 * Nothing here talks to a backend: the three are props, and the story exists
 * to prove they sit where they were meant to — the plan before the run, the
 * sources after it, the pins above the field where what leaves is read.
 */
export const PlanSourcesAndPins: Story = {
  args: {
    state: assistantState(threadWithPlanAndSources),
    pins: PINS,
  },
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement)
    await expect(canvas.getByText("· 1 of 2 done")).toBeVisible()
    await expect(canvas.getByText("Oxyn read 1 object")).toBeVisible()
    await expect(canvas.getByText("Attached to this question")).toBeVisible()
    // The tier of the connection governs what the pins announce, and it is
    // the panel's own `tier` prop — never an application setting (I-04).
    const strip = canvasElement.querySelector(
      '[data-slot="assistant-context-pins"]'
    )
    await expect(strip).toHaveTextContent(
      "Objects — its name, columns, types and indexes"
    )
  },
}

const SETTINGS_EVENT: AiEvent = {
  kind: "agentSettings",
  modes: [],
  currentMode: null,
  options: [
    {
      id: "opt-model-9c1e",
      name: "Model",
      description: null,
      category: "model",
      value: {
        type: "select",
        current: "id-sonnet-7f3a",
        choices: [
          { id: "id-sonnet-7f3a", name: "Sonnet", description: null },
          { id: "id-opus-7f3a", name: "Opus", description: null },
        ],
      },
    },
  ],
}

/**
 * An agent's settings, from the event through the reducer to the footer.
 *
 * They sit above the field, and a provider never gets them: its model is
 * chosen in the header.
 */
export const AgentSettingsAboveTheField: Story = {
  args: {
    selected: destinations[1] ?? null,
    model: null,
    onChangeAgentSetting: fn(async () => {}),
    state: assistantState(
      threadOfEvents([
        {
          question,
          events: [
            { kind: "question", text: question, mentions: [] },
            agentStarted,
            SETTINGS_EVENT,
            { kind: "textDelta", text: answer },
            answered,
          ],
        },
      ])
    ),
  },
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement)
    const model = canvas.getByRole("combobox", { name: "Model: Sonnet" })
    await expect(model).toBeEnabled()
    // Above the field, outside its group: a disabled selector inside it would
    // dim the whole field.
    const field = canvas.getByRole("textbox", {
      name: "Question for the assistant",
    })
    await expect(
      model.compareDocumentPosition(field) & Node.DOCUMENT_POSITION_FOLLOWING
    ).toBeTruthy()
    await expect(
      field.closest('[data-slot="input-group"]')?.contains(model)
    ).toBe(false)
    await expect(canvasElement.innerText).not.toContain("opt-model-9c1e")
  },
}

/**
 * Started when the panel opened: the model, the effort and the rest of what
 * Claude Agent 0.78.0 declares are offered before the first question
 * (measured 2026-09-23, RESEARCH-NOTES).
 */
export const AgentSettingsBeforeTheFirstQuestion: Story = {
  args: {
    selected: destinations[1] ?? null,
    model: null,
    onChangeAgentSetting: fn(async () => {}),
    agentStartup: {
      startup: {
        status: "ready",
        key: "agent:claude",
        label: "Claude Code",
        version: "Claude Agent 0.78.0",
        settings: {
          modes: [],
          currentMode: null,
          options: [
            {
              id: "model",
              name: "Model",
              description: null,
              category: "model",
              value: {
                type: "select",
                current: "default",
                choices: [
                  {
                    id: "default",
                    name: "Default (recommended)",
                    description: null,
                  },
                  { id: "sonnet", name: "Sonnet 5", description: null },
                  { id: "haiku", name: "Haiku 4.5", description: null },
                ],
              },
            },
            {
              id: "effort",
              name: "Effort",
              description: null,
              category: "thoughtLevel",
              value: {
                type: "select",
                current: "default",
                choices: [
                  { id: "default", name: "Default", description: null },
                  { id: "high", name: "High", description: null },
                ],
              },
            },
            {
              id: "fast",
              name: "Fast mode",
              description: null,
              category: "modelConfig",
              value: {
                type: "select",
                current: "off",
                choices: [
                  { id: "on", name: "On", description: null },
                  { id: "off", name: "Off", description: null },
                ],
              },
            },
          ],
        },
      },
      signInStates: {},
      onCancel: fn(),
      onStart: fn(),
      onSignIn: fn(),
      onCopy: fn(() => true),
    },
  },
  play: async ({ canvasElement, args }) => {
    const canvas = within(canvasElement)
    // Nothing asked yet, and the model is already there to choose.
    const model = canvas.getByRole("combobox", {
      name: "Model: Default (recommended)",
    })
    await expect(model).toBeEnabled()
    await userEvent.click(model)
    await userEvent.click(
      await within(document.body).findByRole("option", { name: /Sonnet 5/ })
    )
    await waitFor(() =>
      expect(args.onChangeAgentSetting).toHaveBeenCalledWith({
        kind: "select",
        option: "model",
        choice: "sonnet",
      })
    )
    await waitFor(() =>
      expect(document.querySelector("[data-base-ui-focus-guard]")).toBeNull()
    )
  },
}

/** A restarted agent is another session: its old settings are not offered. */
export const AgentSettingsForgottenAfterRestart: Story = {
  args: {
    selected: destinations[1] ?? null,
    model: null,
    onChangeAgentSetting: fn(async () => {}),
    state: assistantState(
      threadOfEvents([
        {
          question,
          events: [
            { kind: "question", text: question, mentions: [] },
            agentStarted,
            SETTINGS_EVENT,
            answered,
          ],
        },
        {
          question: "And now?",
          events: [
            { kind: "question", text: "And now?", mentions: [] },
            { kind: "memoryReset", reason: "agentRestarted" },
            agentStarted,
            answered,
          ],
        },
      ])
    ),
  },
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement)
    await expect(
      canvas.queryByRole("group", { name: "Agent settings" })
    ).toBeNull()
  },
}

export const ASampleServedOneQuestion: Story = {
  args: {
    state: assistantState(
      threadOfEvents([
        {
          question,
          events: [
            { kind: "question", text: question, mentions: [] },
            { kind: "sampleApproved", rows: 5, columns: 2 },
            started,
            { kind: "textDelta", text: answer },
            answered,
          ],
        },
        {
          question: "And the others?",
          events: [
            { kind: "question", text: "And the others?", mentions: [] },
            { kind: "memoryReset", reason: "sampleNotKept" },
            started,
            { kind: "textDelta", text: answer },
            answered,
          ],
        },
      ])
    ),
  },
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement)
    // Counts, and what became of the sample: nothing of what was sent.
    await expect(
      canvas.getByText(/5 rows and 2 columns of an approved sample/)
    ).toBeVisible()
    await expect(
      canvas.getByText(/An approved sample served one question only/)
    ).toBeVisible()
  },
}

/** A conversation reopened from the workspace: shown whole, remembered by nobody. */
export const ReopenedFromTheWorkspace: Story = {
  args: {
    state: assistantState(
      threadOfEvents([
        {
          question,
          events: [
            { kind: "question", text: question, mentions: [] },
            { kind: "olderNotLoaded" },
            { kind: "textDelta", text: answer },
            {
              kind: "restoredCall",
              tool: "execute_query",
              summary: "1 rows, 1 batches",
              statement: "SELECT count(*) FROM public.clients",
              status: "completed",
              errorClass: null,
              rowsNotKept: true,
            },
            answered,
          ],
        },
        {
          question: "And their emails?",
          events: [
            { kind: "question", text: "And their emails?", mentions: [] },
            { kind: "sampleApproved", rows: 5, columns: 2 },
            { kind: "answerNotKept" },
            answered,
          ],
        },
        {
          question: "And now?",
          events: [
            { kind: "question", text: "And now?", mentions: [] },
            { kind: "memoryReset", reason: "restarted" },
            { kind: "notSaved" },
            started,
            { kind: "textDelta", text: answer },
            answered,
          ],
        },
      ])
    ),
  },
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement)
    await expect(
      canvas.getByText(/used a data sample and was not kept/)
    ).toBeVisible()
    await expect(
      canvas.getByText(/Older exchanges of this conversation/)
    ).toBeVisible()
    await expect(
      canvas.getByText(/not being saved to the workspace/)
    ).toBeVisible()
    await expect(canvas.getByText(/reopened from the workspace/)).toBeVisible()
    // The rows the panel showed then are gone: said, and no grid pretends.
    await expect(
      canvas.getByText(/Result no longer available: the workspace keeps/)
    ).toBeVisible()
    await expect(canvas.queryByRole("grid")).toBeNull()
  },
}

/**
 * The rows an agent's query returned, under its call — for the user only. The
 * grid is drawn by `renderToolRows`, which the feature wires to the backend.
 */
export const AnsweredWithRows: Story = {
  args: {
    state: assistantState(answeredThread),
    renderToolRows: (entry) => (
      <AssistantToolRows
        state={{
          status: "open",
          result: entry.result ?? "",
          columns: invoiceColumns,
          rows: 1,
          truncated: false,
        }}
        fetchPage={syntheticPages(1, 0)}
        onOpenAll={fn()}
      />
    ),
  },
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement)
    const call = canvas.getByRole("region", { name: /Command execute_query/ })
    await expect(
      await within(call).findByRole("grid", { name: "Rows the query returned" })
    ).toBeVisible()
    await expect(within(call).getByText("Acme SA")).toBeVisible()
  },
}
