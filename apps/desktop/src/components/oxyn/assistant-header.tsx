import { HugeiconsIcon } from "@hugeicons/react"
import {
  BubbleChatAddIcon,
  MessageMultiple01Icon,
  RoboticIcon,
} from "@hugeicons/core-free-icons"

import { AssistantAgentStartup } from "@/components/oxyn/assistant-agent-startup"
import type { AgentStartupControls } from "@/components/oxyn/assistant-agent-startup"
import {
  DestinationMark,
  REACH,
} from "@/components/oxyn/assistant-destination-chips"
import { EnvironmentBadge } from "@/components/oxyn/environment-badge"
import { PRIVACY_TIERS } from "@/components/oxyn/privacy-tier"
import { Badge } from "@/components/ui/badge"
import { Button } from "@/components/ui/button"
import { Toggle } from "@/components/ui/toggle"
import {
  Tooltip,
  TooltipContent,
  TooltipTrigger,
} from "@/components/ui/tooltip"
import type { DestinationOption } from "@/features/assistant/availability"
import type { ContextSummary, ProviderReach } from "@/lib/ipc/ai"
import type { Environment, PrivacyTier } from "@/lib/ipc/types"

export type { ModelListState } from "@/components/oxyn/assistant-destination-chips"

/**
 * An agent's destination is not « Unresolved », waiting for a resolution
 * that could come: it is unknowable — a process that sends where it wants
 * (ADR-0026) — and no measure will lift it. It counts as remote all the same,
 * and gets a word of its own rather than the provider's.
 */
const EXTERNAL_AGENT = { label: "Agent-managed", icon: RoboticIcon }

/**
 * « Metadata · Cloud »: what may leave, and where it goes — read together,
 * before speaking (docs/UX-SPEC.md, « Le niveau se lit avant de parler »).
 *
 * The tier's sentence is the badge's description: one hover or one focus
 * away, never in place of the tier itself.
 */
export function AssistantScopeBadge({
  tier,
  reach,
  agent = false,
}: {
  tier: PrivacyTier
  reach: ProviderReach | null
  /** Who answers is an external agent: its destination is unknowable. */
  agent?: boolean
}) {
  const where = agent ? EXTERNAL_AGENT : reach ? REACH[reach] : null
  return (
    <Tooltip>
      <TooltipTrigger
        render={
          <Badge
            variant="outline"
            tabIndex={0}
            data-slot="assistant-scope"
            aria-label={`Privacy tier ${PRIVACY_TIERS[tier].label}${where ? `, destination ${where.label}` : ""}`}
            className="shrink-0 focus-visible:ring-2 focus-visible:ring-ring"
          />
        }
      >
        {where ? (
          <HugeiconsIcon
            icon={where.icon}
            strokeWidth={2}
            data-icon="inline-start"
          />
        ) : null}
        {PRIVACY_TIERS[tier].label}
        {where ? ` · ${where.label}` : ""}
      </TooltipTrigger>
      <TooltipContent className="max-w-72">
        {PRIVACY_TIERS[tier].summary}
      </TooltipContent>
    </Tooltip>
  )
}

function contextLine(context: ContextSummary) {
  const parts = [`${context.relations.toLocaleString()} relations in context`]
  if (context.omittedRelations > 0)
    parts.push(`${context.omittedRelations.toLocaleString()} omitted to fit`)
  if (context.droppedSamples > 0)
    parts.push(
      `${context.droppedSamples.toLocaleString()} row sample(s) withheld by the tier`
    )
  if (context.ignoredMentions > 0)
    parts.push(
      `${context.ignoredMentions.toLocaleString()} mentioned object(s) not found`
    )
  if (context.omittedMentions > 0)
    parts.push(
      `${context.omittedMentions.toLocaleString()} mentioned object(s) named only, over the budget`
    )
  parts.push(`≈ ${context.estimatedTokens.toLocaleString()} tokens`)
  return parts.join(" · ")
}

/**
 * The panel's header: the connection, then one line for what leaves and who
 * receives it — the tier, where it goes, who answers — and what the last
 * question sent. The tier is the connection's, never an application setting
 * (I-04). The selectors that choose who answers sit in the composer, by the
 * question they govern; this line keeps saying who it is.
 */
export function AssistantHeader({
  connectionName,
  environment,
  tier,
  selected,
  model,
  context,
  agentVersion = null,
  historyOpen = false,
  onToggleHistory,
  onNewConversation,
  agentStartup: agentStartupProp = null,
  running,
}: {
  connectionName: string
  environment: Environment
  tier: PrivacyTier
  selected: DestinationOption | null
  /** The model for the selected provider; `null` for an agent. */
  model: string | null
  running: boolean
  /** What the last question sent, counted. */
  context: ContextSummary | null
  /** What the external agent said it is, once started. */
  agentVersion?: string | null
  /** The conversation list is showing instead of the transcript. */
  historyOpen?: boolean
  onToggleHistory?: (open: boolean) => void
  onNewConversation?: () => void
  /** The selected agent's start, before its first question. */
  agentStartup?: AgentStartupControls | null
}) {
  const agent = selected?.kind === "agent"
  const agentStartup = agent ? agentStartupProp : null
  const startup = agentStartup?.startup
  // What the agent said it is: at its last answer, or when it was started
  // for the next one.
  const version =
    agentVersion ?? (startup?.status === "ready" ? startup.version : null)
  const detail = agent ? version : model

  return (
    <header className="flex flex-col gap-1.5 border-b px-3 py-2">
      <div className="flex min-w-0 items-center gap-2">
        <h2 className="shrink-0 text-sm font-medium">Assistant</h2>
        <span className="min-w-0 truncate text-xs text-muted-foreground">
          {connectionName}
        </span>
        <EnvironmentBadge environment={environment} />
        <span className="ml-auto" />
        {onToggleHistory ? (
          <Tooltip>
            <TooltipTrigger
              render={
                <Toggle
                  size="sm"
                  aria-label="Conversations"
                  pressed={historyOpen}
                  onPressedChange={onToggleHistory}
                />
              }
            >
              <HugeiconsIcon icon={MessageMultiple01Icon} strokeWidth={2} />
            </TooltipTrigger>
            <TooltipContent>Conversations</TooltipContent>
          </Tooltip>
        ) : null}
        {onNewConversation ? (
          <Tooltip>
            <TooltipTrigger
              render={
                <Button
                  size="icon-sm"
                  variant="ghost"
                  aria-label="New conversation"
                  onClick={onNewConversation}
                />
              }
            >
              <HugeiconsIcon icon={BubbleChatAddIcon} strokeWidth={2} />
            </TooltipTrigger>
            <TooltipContent>New conversation</TooltipContent>
          </Tooltip>
        ) : null}
      </div>

      <p
        data-slot="assistant-privacy"
        className="flex min-w-0 flex-wrap items-center gap-x-2 gap-y-1 text-xs text-muted-foreground"
      >
        <AssistantScopeBadge
          tier={tier}
          reach={selected?.reach ?? null}
          agent={agent}
        />
        {selected ? (
          <span
            data-slot="assistant-who-answers"
            className="flex min-w-0 items-center gap-1.5"
          >
            <DestinationMark option={selected} />
            <span className="min-w-0 truncate">
              <span className="sr-only">Who answers: </span>
              <span className="font-medium text-foreground">
                {selected.label}
              </span>
              {detail ? ` · ${detail}` : ""}
            </span>
          </span>
        ) : (
          <span>Nobody is chosen to answer.</span>
        )}
        {agent ? (
          <span data-slot="agent-unknowable">
            An external agent: Oxyn cannot see where it sends your question.
          </span>
        ) : null}
      </p>
      {agent && selected.unconfined ? (
        <p className="text-xs text-warning" data-slot="agent-unconfined">
          Oxyn cannot confine this agent, so it cannot stop it from running
          commands or editing files on this machine by itself.
        </p>
      ) : null}
      {selected && !selected.usable && selected.reason ? (
        <p className="text-xs text-destructive">{selected.reason}</p>
      ) : null}

      {/* Hidden while an answer runs: the question then waits for the same
          start, and its own Stop is the one that counts. */}
      {agentStartup && !running ? (
        <AssistantAgentStartup {...agentStartup} />
      ) : null}

      {context ? (
        <p
          className="text-xs text-muted-foreground"
          data-slot="assistant-context"
        >
          {contextLine(context)}
        </p>
      ) : null}
    </header>
  )
}
