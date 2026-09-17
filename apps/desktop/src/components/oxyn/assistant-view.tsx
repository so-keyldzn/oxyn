import * as React from "react"
import { HugeiconsIcon } from "@hugeicons/react"
import { AiChat02Icon } from "@hugeicons/core-free-icons"

import { ApprovalDialog } from "@/components/oxyn/approval-dialog"
import {
  AssistantAgentTool,
  AssistantMemoryReset,
  AssistantPermissionRefused,
  AssistantWaiting,
} from "@/components/oxyn/assistant-agent-activity"
import { AssistantAnswerActions } from "@/components/oxyn/assistant-answer-actions"
import { AssistantAgentSettings } from "@/components/oxyn/assistant-agent-settings"
import type { AgentSettingIntent } from "@/components/oxyn/assistant-agent-settings"
import { AssistantComposer } from "@/components/oxyn/assistant-composer"
import { AssistantContextPins } from "@/components/oxyn/assistant-context-pins"
import type { ContextPin } from "@/components/oxyn/assistant-context-pins"
import { AssistantEnding } from "@/components/oxyn/assistant-ending"
import { AssistantFailure } from "@/components/oxyn/assistant-failure"
import { AssistantHeader } from "@/components/oxyn/assistant-header"
import { AssistantHistory } from "@/components/oxyn/assistant-history"
import { AssistantMarkdown } from "@/components/oxyn/assistant-markdown"
import { AssistantPlan } from "@/components/oxyn/assistant-plan"
import { AssistantQuestion } from "@/components/oxyn/assistant-question"
import { AssistantQueue } from "@/components/oxyn/assistant-queue"
import { AssistantSources } from "@/components/oxyn/assistant-sources"
import { AssistantThinking } from "@/components/oxyn/assistant-thinking"
import { AssistantToolCall } from "@/components/oxyn/assistant-tool-call"
import { AssistantToolDraft } from "@/components/oxyn/assistant-tool-draft"
import { AssistantUsage } from "@/components/oxyn/assistant-usage"
import { Marker } from "@/components/ui/marker"
import { Message, MessageContent } from "@/components/ui/message"
import {
  Empty,
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
import type {
  AssistantEntry,
  DestinationOption,
} from "@/features/assistant/availability"
import type { AssistantState } from "@/features/assistant/conversation-store"
import { activePath, versionsOf } from "@/features/assistant/thread"
import type { ExchangeNode } from "@/features/assistant/thread"
import {
  answerText,
  canContinue,
  outcomeOf,
  saidSomething,
} from "@/features/assistant/transcript"
import type {
  Entry,
  Exchange,
  ToolCallEntry,
} from "@/features/assistant/transcript"
import type {
  AgentProvenance,
  ModelChoice,
  ModelCost,
  ReasoningEffort,
} from "@/lib/ipc/ai"
import type { CatalogAddress, Environment, PrivacyTier } from "@/lib/ipc/types"

const NO_PROVENANCE =
  "This answer cannot be opened in a console: Oxyn cannot record where it came from, and an unmarked statement would look like one you wrote."

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
  /** The price of the selected model, as the provider publishes it. */
  modelCost?: ModelCost | null
  onSelectDestination: (key: string) => void
  onSelectModel: (model: string) => void
  onModelsWanted?: () => void
  /** The efforts the selected model declares; empty hides the selector. */
  efforts?: ReadonlyArray<ReasoningEffort>
  effort?: ReasoningEffort | null
  onSelectEffort?: (effort: ReasoningEffort) => void
  /** Sends or queues; `false` when the backend refused and the draft stays. */
  onAsk: (question: string) => Promise<boolean> | boolean
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
  onSendQueued: (key: string) => void
  onRemoveQueued: (key: string) => void
}

function EntryView({
  entry,
  openSql,
  openSqlDisabledReason,
  onReview,
  onCopy,
}: {
  entry: Entry
  openSql: (sql: string) => void
  openSqlDisabledReason: string | null
  onReview: (entry: ToolCallEntry) => void
  onCopy: (text: string) => Promise<boolean> | boolean
}) {
  switch (entry.kind) {
    case "answer":
      return (
        <Message align="start">
          <MessageContent>
            <AssistantMarkdown
              text={entry.text}
              onOpenSql={openSql}
              openSqlDisabledReason={openSqlDisabledReason}
              onCopy={onCopy}
            />
          </MessageContent>
        </Message>
      )
    case "thinking":
      return <AssistantThinking entry={entry} />
    case "turn":
      return (
        <Marker variant="separator" className="text-xs">
          Turn {entry.turn} / {entry.maxTurns}
        </Marker>
      )
    case "toolDraft":
      return <AssistantToolDraft entry={entry} />
    case "tool":
      return <AssistantToolCall entry={entry} onReview={onReview} />
    case "agentTool":
      return <AssistantAgentTool tool={entry.tool} status={entry.status} />
    case "permissionRefused":
      return (
        <AssistantPermissionRefused
          action={entry.action}
          reason={entry.reason}
        />
      )
    case "memoryReset":
      return <AssistantMemoryReset reason={entry.reason} />
    case "sampleSent":
      return (
        <Marker role="note" className="items-start text-xs">
          <span>
            {entry.rows} {entry.rows === 1 ? "row" : "rows"} and {entry.columns}{" "}
            {entry.columns === 1 ? "column" : "columns"} of an approved sample
            went with this question only. Oxyn keeps none of their values.
          </span>
        </Marker>
      )
    case "rejectedCall":
      return (
        <Marker className="items-start text-xs">
          <span>
            <span className="font-mono">{entry.tool}</span> · refused before the
            bus, nothing ran: {entry.error}
          </span>
        </Marker>
      )
    // Endings and failures are drawn after the exchange, with what they offer.
    case "ended":
    case "failed":
      return null
  }
}

/** One exchange of the shown path: the question, the run, what it offers. */
function ExchangeView({
  node,
  view,
  busy,
  last,
  onReview,
}: {
  node: ExchangeNode
  view: AssistantViewProps
  busy: boolean
  last: boolean
  onReview: (node: number, entry: ToolCallEntry) => void
}) {
  const exchange: Exchange = node.exchange
  const provenance = exchange.started?.provenance ?? null
  const openSqlDisabledReason = provenance === null ? NO_PROVENANCE : null
  const outcome = outcomeOf(exchange)
  const answer = answerText(exchange)
  const waiting =
    exchange.running &&
    !exchange.entries.some(
      (item) => item.kind === "answer" || item.kind === "thinking"
    )

  return (
    <div className="flex flex-col gap-3">
      <AssistantQuestion
        text={exchange.question}
        versions={versionsOf(view.state.thread, node)}
        busy={busy}
        onEdit={(text) => view.onEdit(node, text)}
        onSelectVersion={view.onSelectVersion}
        onCopy={view.onCopy}
      />
      {/* Before the run: what the agent says it will do. It describes intent,
          not authority — what actually went through the bus is drawn by the
          tool calls below, with their connection and their approval (I-07). */}
      <AssistantPlan entries={exchange.plan ?? []} running={exchange.running} />
      {exchange.entries.map((item) => (
        <EntryView
          key={item.key}
          entry={item}
          openSql={(sql) => view.onOpenInConsole(sql, provenance)}
          openSqlDisabledReason={openSqlDisabledReason}
          onReview={(tool) => onReview(node.id, tool)}
          onCopy={view.onCopy}
        />
      ))}
      {waiting ? (
        <AssistantWaiting
          label={
            exchange.stopRequested ? "Stopping…" : "Waiting for the model…"
          }
        />
      ) : null}
      {!exchange.running &&
      outcome?.kind === "ended" &&
      !saidSomething(exchange) ? (
        <Marker role="status" className="text-xs">
          The model answered nothing. This is not a failure.
        </Marker>
      ) : null}

      <AssistantUsage
        usage={exchange.usage}
        contextWindow={exchange.contextWindow}
        cost={view.modelCost ?? null}
      />

      {/* After the run: what Oxyn's own tools read, so the answer can be
          checked against the database rather than believed. */}
      <AssistantSources
        sources={exchange.sources ?? []}
        tier={view.tier}
        onOpenObject={view.onOpenObject}
      />

      {outcome?.kind === "failed" ? (
        <AssistantFailure
          entry={outcome}
          canRetry={!busy}
          signInStates={view.state.signIn}
          onRetry={() => view.onRegenerate(node)}
          onSignIn={view.onSignIn}
          onCopy={view.onCopy}
        />
      ) : null}

      {outcome?.kind === "ended" ? (
        <div className="flex flex-col gap-2">
          <AssistantEnding
            ending={outcome.ending}
            onContinue={
              last && canContinue(exchange)
                ? () => view.onContinue(node)
                : undefined
            }
            continueDisabled={busy}
          />
          {answer !== "" ? (
            <AssistantAnswerActions
              text={answer}
              canRegenerate={!busy}
              onRegenerate={() => view.onRegenerate(node)}
              onCopy={view.onCopy}
            />
          ) : null}
        </div>
      ) : null}
    </div>
  )
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
    onAsk,
    onStop,
    onDecide,
  } = props
  const [reviewing, setReviewing] = React.useState<{
    node: number
    approval: string
  } | null>(null)
  const [historyOpen, setHistoryOpen] = React.useState(false)

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
  const disabledReason =
    entry.status === "disabled"
      ? entry.reason
      : state.opening
        ? "Taking the conversation back…"
        : selected === null || !selected.usable
          ? (selected?.reason ?? "Choose who answers.")
          : null
  const lastExchange = path.at(-1)?.exchange ?? null
  const held = lastExchange ? outcomeOf(lastExchange)?.kind === "failed" : false

  return (
    <section
      aria-label={`Assistant for ${connectionName}`}
      className="flex h-full min-h-0 flex-col bg-background"
    >
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
        onToggleHistory={setHistoryOpen}
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
        />
      ) : (
        <MessageScrollerProvider autoScroll>
          <MessageScroller className="min-h-0 flex-1">
            <MessageScrollerViewport aria-label="Conversation">
              <MessageScrollerContent aria-live="off" className="gap-4 p-3">
                {path.length === 0 ? (
                  <MessageScrollerItem>
                    <Empty className="border-0">
                      <EmptyHeader>
                        <EmptyMedia variant="icon">
                          <HugeiconsIcon icon={AiChat02Icon} strokeWidth={2} />
                        </EmptyMedia>
                        <EmptyTitle>Ask about {connectionName}</EmptyTitle>
                        <EmptyDescription>
                          {entry.status === "disabled"
                            ? entry.reason
                            : "The assistant reads this connection's catalog under its privacy tier. What it proposes arrives in a console as text, and you run it yourself."}
                        </EmptyDescription>
                      </EmptyHeader>
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
            <MessageScrollerButton />
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
            settings={lastAgentSettings(path)}
            disabledReason={
              busy ? "Settings can be changed once the answer is done." : null
            }
            onChange={props.onChangeAgentSetting}
          />
        ) : null}
        <AssistantComposer
          running={state.thread.running !== null}
          stopRequested={lastExchange?.stopRequested ?? false}
          disabledReason={disabledReason}
          onSubmit={onAsk}
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
