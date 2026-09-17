import type { Meta, StoryObj } from "@storybook/react-vite"
import { expect, fn, userEvent, waitFor, within } from "storybook/test"

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
      { kind: "question", text: question },
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

export const Initial: Story = {}

export const Running: Story = {
  args: {
    state: assistantState(
      threadOfEvents(
        [
          {
            question,
            events: [
              { kind: "question", text: question },
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
  },
}

export const ThinkingThenAnswering: Story = {
  args: {
    state: assistantState(
      threadOfEvents([
        {
          question,
          events: [
            { kind: "question", text: question },
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
          { kind: "question", text: question },
          started,
          { kind: "textDelta", text: "1,204." },
          answered,
        ],
      },
      {
        question: "And last month?",
        events: [
          { kind: "question", text: "And last month?" },
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
          events: [{ kind: "question", text: question }, started, answered],
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
            { kind: "question", text: question },
            started,
            {
              kind: "failed",
              message: "provider error: 529 overloaded_error: Overloaded",
              category: "provider",
              retryable: true,
              signIn: null,
              foundElsewhere: null,
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
            { kind: "question", text: question },
            started,
            {
              kind: "failed",
              message:
                "provider error: the stream was cut. A command from this answer may already have been applied: check the data before asking again.",
              category: "provider",
              retryable: false,
              signIn: null,
              foundElsewhere: null,
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
            { kind: "question", text: question },
            {
              kind: "failed",
              message:
                "This connection is local-only, and this provider's endpoint leaves this machine. Nothing was sent.",
              category: "refused",
              retryable: false,
              signIn: null,
              foundElsewhere: null,
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
            { kind: "question", text: question },
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

export const TruncatedOffersToContinue: Story = {
  args: {
    state: assistantState(
      threadOfEvents([
        {
          question,
          events: [
            { kind: "question", text: question },
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
            { kind: "question", text: "Mark invoice 4211 as paid" },
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

    // A message sent meanwhile approves nothing.
    const field = canvas.getByRole("textbox", {
      name: "Question for the assistant",
    })
    await userEvent.type(field, "yes, go ahead{Enter}")
    await expect(args.onAsk).toHaveBeenCalledWith("yes, go ahead")
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
            events: [{ kind: "question", text: question }, started],
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
            { kind: "question", text: question },
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
            { kind: "question", text: question },
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
            { kind: "question", text: question },
            agentStarted,
            SETTINGS_EVENT,
            answered,
          ],
        },
        {
          question: "And now?",
          events: [
            { kind: "question", text: "And now?" },
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
            { kind: "question", text: question },
            { kind: "sampleApproved", rows: 5, columns: 2 },
            started,
            { kind: "textDelta", text: answer },
            answered,
          ],
        },
        {
          question: "And the others?",
          events: [
            { kind: "question", text: "And the others?" },
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
