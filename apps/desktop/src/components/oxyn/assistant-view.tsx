import * as React from "react"
import { HugeiconsIcon } from "@hugeicons/react"
import { SparklesIcon } from "@hugeicons/core-free-icons"

import { ApprovalDialog } from "@/components/oxyn/approval-dialog"
import { AssistantAgentSettings } from "@/components/oxyn/assistant-agent-settings"
import type { AgentSettingIntent } from "@/components/oxyn/assistant-agent-settings"
import type { AgentStartupControls } from "@/components/oxyn/assistant-agent-startup"
import { AssistantComposer } from "@/components/oxyn/assistant-composer"
import type {
  AssistantComposerHandle,
  MentionSource,
} from "@/components/oxyn/assistant-composer"
import { AssistantContextPins } from "@/components/oxyn/assistant-context-pins"
import type { ContextPin } from "@/components/oxyn/assistant-context-pins"
import {
  ExchangeView,
  awaitsReview,
  exchangeCost,
} from "@/components/oxyn/assistant-exchange"
import { AssistantHeader } from "@/components/oxyn/assistant-header"
import type { ModelListState } from "@/components/oxyn/assistant-header"
import { AssistantHistory } from "@/components/oxyn/assistant-history"
import type { OrphanThreadsState } from "@/components/oxyn/assistant-orphans"
import type { ErdRequest } from "@/components/oxyn/assistant-markdown-model"
import { AssistantQueue } from "@/components/oxyn/assistant-queue"
import { Button } from "@/components/ui/button"
import {
  Empty,
  EmptyContent,
  EmptyDescription,
  EmptyHeader,
  EmptyMedia,
  EmptyTitle,
} from "@/components/ui/empty"
import {
  MessageScroller,
  MessageScrollerButton,
  MessageScrollerContent,
  MessageScrollerItem,
  MessageScrollerProvider,
  MessageScrollerViewport,
} from "@/components/ui/message-scroller"
import {
  NO_USABLE_DESTINATION,
  UNREADABLE,
} from "@/features/assistant/availability"
import type {
  AssistantEntry,
  DestinationOption,
} from "@/features/assistant/availability"
import type { AssistantState } from "@/features/assistant/conversation-store"
import { activePath } from "@/features/assistant/thread"
import type { ExchangeNode } from "@/features/assistant/thread"
import { outcomeOf } from "@/features/assistant/transcript"
import type { ToolCallEntry } from "@/features/assistant/transcript"
import type {
  AgentProvenance,
  Mention,
  ModelChoice,
  ModelCost,
  ReasoningEffort,
} from "@/lib/ipc/ai"
import type { CatalogAddress, Environment, PrivacyTier } from "@/lib/ipc/types"

/** Under the field: the whole reason is in the panel above it, once. */
const UNAVAILABLE_SHORT = "Not available on this connection."

/**
 * What the user can do about a closed entry, when there is something to do.
 * Keyed on the reason the backend-facing module wrote, never parsed from it.
 */
const REMEDIES: Record<string, string> = {
  [NO_USABLE_DESTINATION]:
    "Declare a local provider in Settings, or change this connection's privacy tier.",
  [UNREADABLE]:
    "Check the declared providers in Settings; nothing is sent meanwhile.",
}

/**
 * Questions that fill the field, and only fill it: nothing leaves until the
 * user presses send. They name no object, so they hold for any schema.
 */
const EXAMPLES = [
  "What are the main tables here, and how are they related?",
  "Which columns look like foreign keys without a constraint?",
  "Write a query listing the 10 most recent rows of a table.",
]

export interface AssistantViewProps {
  connectionName: string
  environment: Environment
  tier: PrivacyTier
  entry: AssistantEntry
  state: AssistantState
  /** What the user attached to the next question; nothing is sent from it. */
  pins?: ReadonlyArray<ContextPin>
  selected: DestinationOption | null
  model: string | null
  models: ReadonlyArray<ModelChoice> | null
  /**
   * The price of the selected model, as the provider publishes it. An exchange
   * answered by another model shows its own price, or none.
   */
  modelCost?: ModelCost | null
  onSelectDestination: (key: string) => void
  onSelectModel: (model: string) => void
  onModelsWanted?: () => void
  /** The efforts the selected model declares; empty hides the selector. */
  efforts?: ReadonlyArray<ReasoningEffort>
  effort?: ReasoningEffort | null
  onSelectEffort?: (effort: ReasoningEffort) => void
  /**
   * The selected agent's start, before its first question. What it declared
   * then is what the settings offer until an answer reports more.
   */
  agentStartup?: AgentStartupControls | null
  /** The selected provider's model list, while it is loading or failed. */
  modelList?: ModelListState | null
  /**
   * Sends or queues, with what the question names with `@`; `false` when the
   * backend refused and the draft stays.
   */
  onAsk: (
    question: string,
    mentions: Array<Mention>
  ) => Promise<boolean> | boolean
  /** The `@` list of this connection's objects; without it, no list. */
  mentionSource?: MentionSource | null
  onStop: () => void
  onDecide: (node: number, approval: string, approved: boolean) => void
  /** Puts the text in a console, as text. Nothing runs (I-07). */
  onOpenInConsole: (sql: string, provenance: AgentProvenance | null) => void
  /**
   * Opens an object Oxyn read, so the answer can be checked. Runs nothing.
   *
   * Optional: without it the sources stay readable and offer no `Open`, which
   * is what `AssistantSources` already does — an action with nothing behind it
   * is absent, not greyed (docs/UX-SPEC.md).
   */
  onOpenObject?: (address: CatalogAddress) => void
  /**
   * The rows a completed call returned, drawn under it — `AssistantToolRows`
   * wired to the backend. Called only for a call that names a result.
   *
   * Optional: without it a call shows its report alone, as it did before.
   */
  renderToolRows?: (entry: ToolCallEntry) => React.ReactNode
  /**
   * A closed `erd` block of an answer, drawn from the catalog — the feature
   * resolves the names. Optional: without it the block stays code.
   */
  renderErd?: (request: ErdRequest) => React.ReactNode
  /** Unpins locally. Nothing is assembled or sent here. */
  onRemovePin?: (key: string) => void
  /**
   * Asks the external agent to change one of its settings; resolves once the
   * command is accepted, and the agent's next state is what the panel shows.
   *
   * Optional: without it the settings are not drawn — a selector that cannot
   * change anything would be a control that lies about what it offers.
   */
  onChangeAgentSetting?: (change: AgentSettingIntent) => Promise<void>
  onRegenerate: (node: ExchangeNode) => void
  onEdit: (node: ExchangeNode, text: string) => Promise<boolean> | boolean
  onContinue: (node: ExchangeNode) => void
  onSelectVersion: (node: number) => void
  onSignIn: (method: string) => void
  onCopy: (text: string) => Promise<boolean> | boolean
  onNewConversation: () => void
  onOpenThread: (id: string) => void
  onRenameThread: (id: string, title: string) => Promise<void>
  onDeleteThread: (id: string) => Promise<void>
  onReloadHistory: () => void
  /** The conversations of deleted connections, shown read-only in the history. */
  orphans?: OrphanThreadsState
  /** The history list was just shown: what it holds from elsewhere is read again. */
  onHistoryShown?: () => void
  onSendQueued: (key: string) => void
  onRemoveQueued: (key: string) => void
}

/**
 * The one thing the panel announces, in one live region.
 *
 * Every step of a run used to be its own `status`, and a screen reader read
 * each marker as it arrived — a tool, a wait, an ending — over the answer
 * itself. A state change is what the user needs to hear; the transcript is
 * there to be read.
 */
function panelStatus(path: ReadonlyArray<ExchangeNode>) {
  const last = path.at(-1)?.exchange
  if (!last) return ""
  if (path.some((node) => awaitsReview(node.exchange)))
    return "Needs your review"
  if (last.running) return "Answering…"
  const outcome = outcomeOf(last)
  if (outcome?.kind === "failed") return "Failed"
  if (outcome?.kind === "ended") return "Answer ready"
  return ""
}

/** What the first empty screen says, for whoever answers. */
function introduction(selected: DestinationOption | null) {
  if (selected?.kind === "agent") {
    return `${selected.label} receives your question as you type it, and reads this connection only through Oxyn's tools, which apply its privacy tier on every call. What it proposes arrives in a console as text, and you run it yourself.`
  }
  return "The assistant reads this connection's catalog under its privacy tier. What it proposes arrives in a console as text, and you run it yourself."
}

/**
 * The assistant panel, drawn from props only.
 *
 * `absent` draws nothing at all: without a declared provider the AI workspace
 * does not exist (docs/UX-SPEC.md). `disabled` draws the panel with its reason
 * and no way to send.
 */
export function AssistantView(props: AssistantViewProps) {
  const {
    connectionName,
    environment,
    tier,
    entry,
    state,
    selected,
    model,
    models,
    onSelectDestination,
    onSelectModel,
    onModelsWanted,
    efforts,
    effort,
    onSelectEffort,
    agentStartup = null,
    modelList = null,
    onAsk,
    onStop,
    onDecide,
  } = props
  const [reviewing, setReviewing] = React.useState<{
    node: number
    approval: string
  } | null>(null)
  const [historyOpen, setHistoryOpen] = React.useState(false)
  const composer = React.useRef<AssistantComposerHandle>(null)
  // The agent started when the panel opened, before any question: what it
  // declared then is what the selector offers until an answer reports more.
  const startedSettings =
    agentStartup?.startup.status === "ready"
      ? agentStartup.startup.settings
      : null

  if (entry.status === "absent") return null

  const path = activePath(state.thread)
  const busy = state.thread.running !== null || state.sending
  const review = path
    .flatMap((node) =>
      node.exchange.entries.map((item) => ({ node: node.id, item }))
    )
    .find(
      (found): found is { node: number; item: ToolCallEntry } =>
        found.item.kind === "tool" &&
        found.item.approval?.id === reviewing?.approval
    )
  const unavailable = entry.status === "disabled"
  const disabledReason = unavailable
    ? UNAVAILABLE_SHORT
    : state.opening
      ? "Taking the conversation back…"
      : selected === null || !selected.usable
        ? (selected?.reason ?? "Choose who answers.")
        : null
  const lastExchange = path.at(-1)?.exchange ?? null
  const held = lastExchange ? outcomeOf(lastExchange)?.kind === "failed" : false
  const remedy = unavailable ? (REMEDIES[entry.reason] ?? null) : null

  return (
    <section
      aria-label={`Assistant for ${connectionName}`}
      className="flex h-full min-h-0 flex-col bg-background"
    >
      <p data-slot="assistant-status" role="status" className="sr-only">
        {panelStatus(path)}
      </p>
      <AssistantHeader
        connectionName={connectionName}
        environment={environment}
        tier={tier}
        destinations={entry.destinations}
        selected={selected}
        model={model}
        models={models}
        running={busy}
        context={lastContext(path)}
        agentVersion={lastExchange?.started?.destination.agentVersion ?? null}
        historyOpen={historyOpen}
        onSelectDestination={onSelectDestination}
        onSelectModel={onSelectModel}
        onModelsWanted={onModelsWanted}
        efforts={efforts}
        effort={effort}
        onSelectEffort={onSelectEffort}
        agentStartup={agentStartup}
        modelList={modelList}
        onToggleHistory={(open: boolean) => {
          setHistoryOpen(open)
          if (open) props.onHistoryShown?.()
        }}
        onNewConversation={() => {
          setHistoryOpen(false)
          props.onNewConversation()
        }}
      />

      {historyOpen ? (
        <AssistantHistory
          history={state.history}
          currentId={state.thread.id}
          onNew={() => {
            setHistoryOpen(false)
            props.onNewConversation()
          }}
          onOpen={(id) => {
            setHistoryOpen(false)
            props.onOpenThread(id)
          }}
          onRename={props.onRenameThread}
          onDelete={props.onDeleteThread}
          onReload={props.onReloadHistory}
          orphans={props.orphans}
        />
      ) : (
        <MessageScrollerProvider autoScroll>
          {/* While the button to the end shows, the conversation gives it a
              strip of its own below the viewport: floating over the text, it
              hid the words of whatever it covered. */}
          <MessageScroller className="min-h-0 flex-1 has-[[data-slot=message-scroller-button][data-active=true]]:pb-9">
            <MessageScrollerViewport aria-label="Conversation">
              <MessageScrollerContent aria-live="off" className="gap-4 p-3">
                {path.length === 0 ? (
                  <MessageScrollerItem>
                    <Empty className="border-0">
                      <EmptyHeader>
                        <EmptyMedia variant="icon">
                          <HugeiconsIcon icon={SparklesIcon} strokeWidth={2} />
                        </EmptyMedia>
                        <EmptyTitle>
                          {unavailable
                            ? "Not available on this connection"
                            : `Ask about ${connectionName}`}
                        </EmptyTitle>
                        <EmptyDescription>
                          {unavailable ? entry.reason : introduction(selected)}
                        </EmptyDescription>
                        {remedy ? (
                          <EmptyDescription data-slot="assistant-remedy">
                            {remedy}
                          </EmptyDescription>
                        ) : null}
                      </EmptyHeader>
                      {disabledReason === null ? (
                        <EmptyContent>
                          <ul
                            aria-label="Example questions"
                            className="flex w-full flex-col gap-1.5"
                          >
                            {EXAMPLES.map((example) => (
                              <li key={example}>
                                <Button
                                  variant="outline"
                                  size="sm"
                                  className="h-auto w-full justify-start py-1.5 text-left whitespace-normal"
                                  onClick={() =>
                                    composer.current?.fill(example)
                                  }
                                >
                                  {example}
                                </Button>
                              </li>
                            ))}
                          </ul>
                        </EmptyContent>
                      ) : null}
                    </Empty>
                  </MessageScrollerItem>
                ) : null}
                {path.map((node, index) => (
                  <MessageScrollerItem
                    key={node.id}
                    messageId={String(node.id)}
                    scrollAnchor
                  >
                    <ExchangeView
                      node={node}
                      view={props}
                      busy={busy}
                      last={index === path.length - 1}
                      cost={exchangeCost(node.exchange, props)}
                      onReview={(id, tool) =>
                        setReviewing(
                          tool.approval
                            ? { node: id, approval: tool.approval.id }
                            : null
                        )
                      }
                    />
                  </MessageScrollerItem>
                ))}
              </MessageScrollerContent>
            </MessageScrollerViewport>
            <MessageScrollerButton className="data-[direction=end]:bottom-1" />
          </MessageScroller>
        </MessageScrollerProvider>
      )}

      <div className="flex flex-col gap-2 border-t p-3">
        <AssistantQueue
          queue={state.thread.queue}
          held={held}
          onSendNow={props.onSendQueued}
          onRemove={props.onRemoveQueued}
        />
        {/* Above the field, because this is where what will leave the machine
            is read — before pressing, not after (ADR-0006).

            Drawn only when there is a way to unpin: an attachment the user
            cannot detach is a control that lies about what it offers. */}
        {props.onRemovePin ? (
          <AssistantContextPins
            pins={props.pins ?? []}
            tier={tier}
            onRemove={props.onRemovePin}
          />
        ) : null}
        {state.askError ? (
          <p role="alert" data-selectable className="text-xs text-destructive">
            {state.askError}
          </p>
        ) : null}
        {/* Above the field rather than inside its group: `InputGroup` dims
            every child at 50 % as soon as one control in it is `disabled`, and
            these are disabled for as long as an answer runs. Only for an agent:
            a provider's model is chosen in the header. */}
        {selected?.kind === "agent" && props.onChangeAgentSetting ? (
          <AssistantAgentSettings
            settings={lastAgentSettings(path) ?? startedSettings}
            disabledReason={
              busy ? "Settings can be changed once the answer is done." : null
            }
            onChange={props.onChangeAgentSetting}
          />
        ) : null}
        <AssistantComposer
          ref={composer}
          running={state.thread.running !== null}
          stopRequested={lastExchange?.stopRequested ?? false}
          disabledReason={disabledReason}
          // The question goes to the current conversation: it is shown again
          // first, so nothing is sent into a thread the user cannot see.
          mentions={props.mentionSource ?? null}
          onSubmit={(question, mentions) => {
            setHistoryOpen(false)
            return onAsk(question, mentions)
          }}
          onStop={onStop}
        />
      </div>

      <ApprovalDialog
        approval={
          review?.item.approval
            ? {
                command: review.item.approval.id,
                reason: review.item.approval.reason,
                preview: {
                  statement: review.item.approval.statement,
                  connection: review.item.approval.connection,
                  estimatedRows: null,
                },
              }
            : null
        }
        connectionName={review?.item.approval?.connection ?? connectionName}
        // Unknown means production: the stricter reading (I-02).
        environment={review?.item.approval?.environment ?? "production"}
        actor={{
          kind: "agent",
          name: lastExchange?.started?.destination.label ?? "the assistant",
        }}
        deciding={review?.item.state === "deciding"}
        onDecide={(approved) => {
          const target = review
          setReviewing(null)
          if (target?.item.approval) {
            onDecide(target.node, target.item.approval.id, approved)
          }
        }}
      />
    </section>
  )
}

/**
 * The agent's settings as it last reported them on the shown path.
 *
 * They are the agent session's, not the exchange's: a new question starts
 * with none until the agent speaks again, and the selector must not blink out
 * in between. But a restarted agent or another destination is another session,
 * and settings from before it would offer ids the new one does not know.
 */
function lastAgentSettings(path: ReadonlyArray<ExchangeNode>) {
  for (let index = path.length - 1; index >= 0; index -= 1) {
    const exchange = path[index]?.exchange
    if (!exchange) continue
    if (exchange.agentSettings) return exchange.agentSettings
    const newSession = exchange.entries.some(
      (entry) =>
        entry.kind === "memoryReset" &&
        (entry.reason === "agentRestarted" ||
          entry.reason === "destinationChanged")
    )
    if (newSession) return null
  }
  return null
}

/** The context of the most recent exchange that assembled one. */
function lastContext(path: ReadonlyArray<ExchangeNode>) {
  for (let index = path.length - 1; index >= 0; index -= 1) {
    const context = path[index]?.exchange.started?.context
    if (context) return context
  }
  return null
}
