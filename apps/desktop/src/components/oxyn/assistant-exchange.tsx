import * as React from "react"

import {
  AssistantAgentTool,
  AssistantCatalogRead,
  AssistantMemoryReset,
  AssistantPermissionRefused,
  AssistantWaiting,
} from "@/components/oxyn/assistant-agent-activity"
import {
  AssistantAnswerActions,
  AssistantAnswerMenu,
} from "@/components/oxyn/assistant-answer-actions"
import { AssistantEnding } from "@/components/oxyn/assistant-ending"
import { AssistantFailure } from "@/components/oxyn/assistant-failure"
import { AssistantMarkdown } from "@/components/oxyn/assistant-markdown"
import { AssistantPlan } from "@/components/oxyn/assistant-plan"
import { AssistantQuestion } from "@/components/oxyn/assistant-question"
import { AssistantSources } from "@/components/oxyn/assistant-sources"
import { AssistantThinking } from "@/components/oxyn/assistant-thinking"
import {
  AssistantRestoredCall,
  AssistantToolCall,
} from "@/components/oxyn/assistant-tool-call"
import { AssistantToolDraft } from "@/components/oxyn/assistant-tool-draft"
import { AssistantUsage } from "@/components/oxyn/assistant-usage"
import type { AssistantViewProps } from "@/components/oxyn/assistant-view"
import { Marker, MarkerContent } from "@/components/ui/marker"
import { Message, MessageContent } from "@/components/ui/message"
import type { ExchangeNode, Versions } from "@/features/assistant/thread"
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

/**
 * What an exchange can ask of the panel. The view hands the same object to
 * every exchange for as long as it is mounted, so an exchange whose node did
 * not change is not drawn again while another one streams.
 */
export interface ExchangeActions {
  onOpenObject?: AssistantViewProps["onOpenObject"]
  onEdit: AssistantViewProps["onEdit"]
  onSelectVersion: AssistantViewProps["onSelectVersion"]
  onCopy: AssistantViewProps["onCopy"]
  onOpenInConsole: AssistantViewProps["onOpenInConsole"]
  onRegenerate: AssistantViewProps["onRegenerate"]
  onContinue: AssistantViewProps["onContinue"]
  onSignIn: AssistantViewProps["onSignIn"]
  onReview: (node: number, entry: ToolCallEntry) => void
}

const EntryView = React.memo(function EntryView({
  entry,
  openSql,
  openSqlDisabledReason,
  onReview,
  onCopy,
  renderToolRows,
  renderErd,
  identifierQuote,
}: {
  entry: Entry
  openSql: (sql: string) => void
  openSqlDisabledReason: string | null
  onReview: (entry: ToolCallEntry) => void
  onCopy: (text: string) => Promise<boolean> | boolean
  renderToolRows?: AssistantViewProps["renderToolRows"]
  identifierQuote?: AssistantViewProps["identifierQuote"]
  renderErd?: AssistantViewProps["renderErd"]
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
              renderErd={renderErd}
              identifierQuote={identifierQuote}
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
    case "catalog":
      return <AssistantCatalogRead entry={entry} />
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
      return <AssistantRestoredCall entry={entry} />

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
})

interface ExchangeViewProps {
  node: ExchangeNode
  versions: Versions
  busy: boolean
  last: boolean
  /** The price of **this** exchange's model; `null` shows no cost. */
  cost: ModelCost | null
  tier: AssistantViewProps["tier"]
  signInStates: AssistantViewProps["state"]["signIn"]
  identifierQuote?: AssistantViewProps["identifierQuote"]
  renderToolRows?: AssistantViewProps["renderToolRows"]
  renderErd?: AssistantViewProps["renderErd"]
  actions: ExchangeActions
}

// `versions` is computed afresh by each render of the view: equal is enough.
function sameExchangeProps(
  before: ExchangeViewProps,
  after: ExchangeViewProps
) {
  const keys = Object.keys({ ...before, ...after }) as Array<
    keyof ExchangeViewProps
  >
  return keys.every((key) =>
    key === "versions"
      ? before.versions.position === after.versions.position &&
        before.versions.count === after.versions.count &&
        before.versions.previous === after.versions.previous &&
        before.versions.next === after.versions.next
      : Object.is(before[key], after[key])
  )
}

/** One exchange of the shown path: the question, the run, what it offers. */
export const ExchangeView = React.memo(function ExchangeView({
  node,
  versions,
  busy,
  last,
  cost,
  tier,
  signInStates,
  identifierQuote,
  renderToolRows,
  renderErd,
  actions,
}: ExchangeViewProps) {
  const exchange: Exchange = node.exchange
  const provenance = exchange.started?.provenance ?? null
  const openSqlDisabledReason = provenance === null ? NO_PROVENANCE : null
  // Stable while the exchange streams, so the entries already drawn are not.
  const openSql = React.useCallback(
    (sql: string) => actions.onOpenInConsole(sql, provenance),
    [actions, provenance]
  )
  const review = React.useCallback(
    (tool: ToolCallEntry) => actions.onReview(node.id, tool),
    [actions, node.id]
  )
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
        mentions={exchange.mentions}
        onOpenObject={actions.onOpenObject}
        versions={versions}
        busy={busy}
        onEdit={(text) => actions.onEdit(node, text)}
        onSelectVersion={actions.onSelectVersion}
        onCopy={actions.onCopy}
      />
      {/* Before the run: what the agent says it will do. It describes intent,
          not authority — what actually went through the bus is drawn by the
          tool calls below, with their connection and their approval (I-07). */}
      <AssistantPlan entries={exchange.plan ?? []} running={exchange.running} />
      {exchange.entries.map((item) => {
        const entry = (
          <EntryView
            key={item.key}
            entry={item}
            openSql={openSql}
            openSqlDisabledReason={openSqlDisabledReason}
            onReview={review}
            onCopy={actions.onCopy}
            renderToolRows={renderToolRows}
            renderErd={renderErd}
            identifierQuote={identifierQuote}
          />
        )
        // A right click on any part of the answer acts on the whole answer,
        // as the buttons under it do.
        return item.kind === "answer" && answer !== "" ? (
          <AssistantAnswerMenu
            key={item.key}
            text={answer}
            answering={busy}
            onCopy={actions.onCopy}
            onRegenerate={() => actions.onRegenerate(node)}
          >
            {entry}
          </AssistantAnswerMenu>
        ) : (
          entry
        )
      })}
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
        tier={tier}
        onOpenObject={actions.onOpenObject}
      />

      {outcome?.kind === "failed" ? (
        <AssistantFailure
          entry={outcome}
          canRetry={!busy}
          signInStates={signInStates}
          onRetry={() => actions.onRegenerate(node)}
          onSignIn={actions.onSignIn}
          onCopy={actions.onCopy}
        />
      ) : null}

      {outcome?.kind === "ended" ? (
        <div className="flex flex-col gap-2">
          <AssistantEnding
            ending={outcome.ending}
            awaitingReview={awaitsReview(exchange)}
            onContinue={
              last && canContinue(exchange)
                ? () => actions.onContinue(node)
                : undefined
            }
            continueDisabled={busy}
          />
          {answer !== "" ? (
            <AssistantAnswerActions
              text={answer}
              canRegenerate={!busy}
              onRegenerate={() => actions.onRegenerate(node)}
              onCopy={actions.onCopy}
            />
          ) : null}
        </div>
      ) : null}
    </div>
  )
}, sameExchangeProps)
