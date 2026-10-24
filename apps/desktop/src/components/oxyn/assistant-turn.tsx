import type * as React from "react"

import { DestinationMark } from "@/components/oxyn/assistant-destination-chips"
import { AssistantUsage } from "@/components/oxyn/assistant-usage"
import type { ContextWindow, Usage } from "@/features/assistant/transcript"
import type { Destination, ModelCost } from "@/lib/ipc/ai"

/**
 * Who answered this turn, at its start: the mark, the name, and the model or
 * the agent's version as recorded when the exchange started — not today's
 * selection, which may have changed since.
 */
export function AssistantTurnIdentity({
  destination,
  brand,
}: {
  destination: Destination
  /** The destination's mark while it is still declared; `null` otherwise. */
  brand: string | null
}) {
  const agent = destination.kind === "agent"
  const detail = agent ? destination.agentVersion : destination.model
  return (
    <p
      data-slot="assistant-turn-identity"
      className="flex min-w-0 items-center gap-1.5 text-xs text-muted-foreground"
    >
      <DestinationMark
        option={{
          kind: agent ? "agent" : "provider",
          brand: brand ?? undefined,
        }}
      />
      <span className="min-w-0 truncate">
        <span className="font-medium text-foreground">{destination.label}</span>
        {detail ? ` · ${detail}` : ""}
      </span>
    </p>
  )
}

/**
 * The quiet end of a turn, on one row: how it ended when that asks nothing of
 * the reader, what it consumed, and what can be done with the answer — the
 * actions keep their names and their keyboard reach.
 */
export function AssistantTurnMeta({
  ending,
  usage,
  contextWindow,
  cost,
  actions,
}: {
  /** The ending, already drawn quiet; `null` when it has a line of its own. */
  ending: React.ReactNode
  usage: Usage | null
  contextWindow: ContextWindow | null
  cost: ModelCost | null
  actions: React.ReactNode
}) {
  const consumed = usage !== null || contextWindow !== null
  if (!ending && !consumed && !actions) return null
  return (
    <div
      data-slot="assistant-turn-meta"
      className="flex min-w-0 items-start gap-2 text-xs text-muted-foreground"
    >
      {/* The words wrap within their column; the actions keep their place
          at the end of the first line. */}
      <div className="flex min-h-6 min-w-0 flex-1 flex-wrap items-center gap-x-1.5 gap-y-0.5">
        {ending}
        {ending && consumed ? <span aria-hidden>·</span> : null}
        <AssistantUsage
          usage={usage}
          contextWindow={contextWindow}
          cost={cost}
        />
      </div>
      {actions ? <div className="shrink-0">{actions}</div> : null}
    </div>
  )
}
