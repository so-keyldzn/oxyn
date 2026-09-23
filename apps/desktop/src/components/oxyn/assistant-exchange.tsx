import {
  AssistantAgentTool,
  AssistantMemoryReset,
  AssistantPermissionRefused,
  AssistantWaiting,
} from "@/components/oxyn/assistant-agent-activity"
import { AssistantAnswerActions } from "@/components/oxyn/assistant-answer-actions"
import { AssistantEnding } from "@/components/oxyn/assistant-ending"
import { AssistantFailure } from "@/components/oxyn/assistant-failure"
import { AssistantMarkdown } from "@/components/oxyn/assistant-markdown"
import { AssistantPlan } from "@/components/oxyn/assistant-plan"
import { AssistantQuestion } from "@/components/oxyn/assistant-question"
import { AssistantSources } from "@/components/oxyn/assistant-sources"
import { AssistantThinking } from "@/components/oxyn/assistant-thinking"
import { AssistantToolCall } from "@/components/oxyn/assistant-tool-call"
import { AssistantToolDraft } from "@/components/oxyn/assistant-tool-draft"
import { AssistantUsage } from "@/components/oxyn/assistant-usage"
import type { AssistantViewProps } from "@/components/oxyn/assistant-view"
import { Marker, MarkerContent } from "@/components/ui/marker"
import { Message, MessageContent } from "@/components/ui/message"
import { versionsOf } from "@/features/assistant/thread"
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
import type { ModelCost } from "@/lib/ipc/ai"

const NO_PROVENANCE =
  "This answer cannot be opened in a console: Oxyn cannot record where it came from, and an unmarked statement would look like one you wrote."

/** Whether a command of this exchange still waits for the user's decision. */
export function awaitsReview(exchange: Exchange) {
  return exchange.entries.some(
    (item) =>
      item.kind === "tool" &&
      item.approval !== null &&
      (item.state === "awaitingApproval" || item.state === "deciding")
  )
}

/**
 * The price that applies to one exchange, or `null` when it is not known.
 *
 * Only the selected provider's model list carries prices, so an exchange
 * answered elsewhere — another provider, an agent, a model no longer listed —
 * shows no cost rather than today's price for something else.
 */
export function exchangeCost(
  exchange: Exchange,
  view: Pick<AssistantViewProps, "selected" | "model" | "models" | "modelCost">
): ModelCost | null {
  const destination = exchange.started?.destination
  if (
    !destination ||
    destination.kind !== "provider" ||
    destination.model === null ||
    view.selected?.kind !== "provider" ||
    destination.label !== view.selected.label
  ) {
    return null
  }
  if (destination.model === view.model) return view.modelCost ?? null
  return (
    view.models?.find((choice) => choice.id === destination.model)?.cost ?? null
  )
}

function EntryView({
  entry,
  openSql,
  openSqlDisabledReason,
  onReview,
  onCopy,
  renderToolRows,
}: {
  entry: Entry
  openSql: (sql: string) => void
  openSqlDisabledReason: string | null
  onReview: (entry: ToolCallEntry) => void
  onCopy: (text: string) => Promise<boolean> | boolean
  renderToolRows?: AssistantViewProps["renderToolRows"]
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
          <MarkerContent>
            Turn {entry.turn} / {entry.maxTurns}
          </MarkerContent>
        </Marker>
      )
    case "toolDraft":
      return <AssistantToolDraft entry={entry} />
    case "tool":
      return (
        <AssistantToolCall
          entry={entry}
          onReview={onReview}
          rows={
            entry.state === "completed" && entry.result !== null
              ? renderToolRows?.(entry)
              : null
          }
        />
      )
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
    case "notSaved":
      return (
        <Marker role="note" className="items-start text-xs">
          <MarkerContent>
            This conversation is not being saved to the workspace. The assistant
            still answers; nothing of it will be here next time.
          </MarkerContent>
        </Marker>
      )
    case "olderNotLoaded":
      return (
        <Marker role="note" className="items-start text-xs">
          <MarkerContent>
            Older exchanges of this conversation are in the workspace and not
            loaded here.
          </MarkerContent>
        </Marker>
      )
    case "answerNotKept":
      return (
        <Marker role="note" className="items-start text-xs">
          <MarkerContent>
            This answer used a data sample and was not kept. The workspace holds
            the question and the size of the sample, nothing else.
          </MarkerContent>
        </Marker>
      )
    case "restoredCall":
      return (
        <Marker className="items-start text-xs">
          <MarkerContent>
            <span className="font-mono">{entry.tool}</span> · {entry.summary}
            {entry.statement ? (
              <span className="mt-1 block font-mono break-all opacity-80">
                {entry.statement}
              </span>
            ) : null}
            {entry.rowsNotKept ? (
              <span className="mt-1 block">
                Result no longer available: the workspace keeps the statement,
                never its rows. Nothing is rerun.
              </span>
            ) : null}
          </MarkerContent>
        </Marker>
      )
    case "sampleSent":
      return (
        <Marker role="note" className="items-start text-xs">
          <MarkerContent>
            {entry.rows} {entry.rows === 1 ? "row" : "rows"} and {entry.columns}{" "}
            {entry.columns === 1 ? "column" : "columns"} of an approved sample
            went with this question only. Oxyn keeps none of their values.
          </MarkerContent>
        </Marker>
      )
    case "rejectedCall":
      return (
        <Marker className="items-start text-xs">
          <MarkerContent>
            <span className="font-mono">{entry.tool}</span> · refused before the
            bus, nothing ran: {entry.error}
          </MarkerContent>
        </Marker>
      )
    // Endings and failures are drawn after the exchange, with what they offer.
    case "ended":
    case "failed":
      return null
  }
}

/** One exchange of the shown path: the question, the run, what it offers. */
export function ExchangeView({
  node,
  view,
  busy,
  last,
  cost,
  onReview,
}: {
  node: ExchangeNode
  view: AssistantViewProps
  busy: boolean
  last: boolean
  /** The price of **this** exchange's model; `null` shows no cost. */
  cost: ModelCost | null
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
          renderToolRows={view.renderToolRows}
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
        <Marker className="text-xs">
          <MarkerContent>
            The model answered nothing. This is not a failure.
          </MarkerContent>
        </Marker>
      ) : null}

      <AssistantUsage
        usage={exchange.usage}
        contextWindow={exchange.contextWindow}
        cost={cost}
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
            awaitingReview={awaitsReview(exchange)}
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
