import { HugeiconsIcon } from "@hugeicons/react"
import {
  BubbleChatAddIcon,
  CloudIcon,
  ComputerIcon,
  HelpCircleIcon,
  MessageMultiple01Icon,
  RoboticIcon,
} from "@hugeicons/core-free-icons"

import { AssistantAgentStartup } from "@/components/oxyn/assistant-agent-startup"
import type { AgentStartupControls } from "@/components/oxyn/assistant-agent-startup"
import { AssistantReasoningEffort } from "@/components/oxyn/assistant-reasoning-effort"
import { EnvironmentBadge } from "@/components/oxyn/environment-badge"
import { PRIVACY_TIERS } from "@/components/oxyn/privacy-tier"
import { Badge } from "@/components/ui/badge"
import { Button } from "@/components/ui/button"
import { Spinner } from "@/components/ui/spinner"
import { Toggle } from "@/components/ui/toggle"
import {
  Tooltip,
  TooltipContent,
  TooltipTrigger,
} from "@/components/ui/tooltip"
import {
  Select,
  SelectContent,
  SelectGroup,
  SelectItem,
  SelectLabel,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select"
import type { DestinationOption } from "@/features/assistant/availability"
import type {
  ContextSummary,
  ModelChoice,
  ProviderReach,
  ReasoningEffort,
} from "@/lib/ipc/ai"
import type { Environment, PrivacyTier } from "@/lib/ipc/types"

const REACH: Record<ProviderReach, { label: string; icon: typeof CloudIcon }> =
  {
    local: { label: "This machine", icon: ComputerIcon },
    remote: { label: "Cloud", icon: CloudIcon },
    // Counts as remote; said as what it is.
    unresolved: { label: "Unresolved", icon: HelpCircleIcon },
  }

/**
 * An agent's destination is not « Unresolved », waiting for a resolution
 * that could come: it is unknowable — a process that sends where it wants
 * (ADR-0026) — and no measure will lift it. It counts as remote all the same,
 * and gets a word of its own rather than the provider's.
 */
const EXTERNAL_AGENT = { label: "Agent-managed", icon: RoboticIcon }

/** The list of models of the selected provider, while it is not there. */
export type ModelListState =
  { status: "loading" } | { status: "error"; message: string }

/**
 * « Metadata · Cloud »: what may leave, and where it goes — read together,
 * before speaking (docs/UX-SPEC.md, « Le niveau se lit avant de parler »).
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
    <Badge
      variant="outline"
      data-slot="assistant-scope"
      aria-label={`Privacy tier ${PRIVACY_TIERS[tier].label}${where ? `, destination ${where.label}` : ""}`}
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
    </Badge>
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
 * The panel's header: the connection, its tier in words, who answers, and
 * what the last question sent. The tier is the connection's, never an
 * application setting (I-04).
 */
export function AssistantHeader({
  connectionName,
  environment,
  tier,
  destinations,
  selected,
  model,
  models,
  running,
  context,
  agentVersion = null,
  historyOpen = false,
  onSelectDestination,
  onSelectModel,
  onModelsWanted,
  efforts = [],
  effort = null,
  onSelectEffort,
  onToggleHistory,
  onNewConversation,
  agentStartup: agentStartupProp = null,
  modelList: modelListProp = null,
}: {
  connectionName: string
  environment: Environment
  tier: PrivacyTier
  destinations: ReadonlyArray<DestinationOption>
  selected: DestinationOption | null
  /** The model for the selected provider; `null` for an agent. */
  model: string | null
  /** What the provider listed, when asked; `null` before. */
  models: ReadonlyArray<ModelChoice> | null
  running: boolean
  /** What the last question sent, counted. */
  context: ContextSummary | null
  /** What the external agent said it is, once started. */
  agentVersion?: string | null
  /** The conversation list is showing instead of the transcript. */
  historyOpen?: boolean
  onSelectDestination: (key: string) => void
  onSelectModel: (model: string) => void
  /** Opening the model list; the list itself is asked for by the caller. */
  onModelsWanted?: () => void
  /** The efforts the selected model declares; empty hides the selector. */
  efforts?: ReadonlyArray<ReasoningEffort>
  /** The effort chosen for this provider; `null` leaves the provider's default. */
  effort?: ReasoningEffort | null
  onSelectEffort?: (effort: ReasoningEffort) => void
  onToggleHistory?: (open: boolean) => void
  onNewConversation?: () => void
  /** The selected agent's start, before its first question. */
  agentStartup?: AgentStartupControls | null
  /** The selected provider's model list, while it is loading or failed. */
  modelList?: ModelListState | null
}) {
  const agentStartup = selected?.kind === "agent" ? agentStartupProp : null
  const modelList = selected?.kind === "provider" ? modelListProp : null
  const startup = agentStartup?.startup
  // What the agent said it is: at its last answer, or when it was started
  // for the next one.
  const version =
    agentVersion ?? (startup?.status === "ready" ? startup.version : null)
  const providers = destinations.filter((option) => option.kind === "provider")
  const agents = destinations.filter((option) => option.kind === "agent")
  const modelIds = Array.from(
    new Set([
      ...(selected?.model ? [selected.model] : []),
      ...(model ? [model] : []),
      ...(models ?? []).map((choice) => choice.id),
    ])
  )

  return (
    <header className="flex flex-col gap-2 border-b px-3 py-2.5">
      <div className="flex min-w-0 items-center gap-2">
        <h2 className="truncate text-sm font-medium">Assistant</h2>
        <span className="truncate text-xs text-muted-foreground">
          {connectionName}
        </span>
        <EnvironmentBadge environment={environment} />
        <span className="ml-auto" />
        <AssistantScopeBadge
          tier={tier}
          reach={selected?.reach ?? null}
          agent={selected?.kind === "agent"}
        />
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
      <p className="text-xs text-muted-foreground">
        {PRIVACY_TIERS[tier].summary}
      </p>

      <div className="flex flex-wrap items-center gap-2">
        <Select
          value={selected?.key ?? null}
          items={destinations.map((option) => ({
            value: option.key,
            label: option.label,
          }))}
          disabled={running}
          onValueChange={(value) => {
            if (typeof value === "string") onSelectDestination(value)
          }}
        >
          <SelectTrigger
            size="sm"
            aria-label="Who answers"
            className="min-w-40"
          >
            <SelectValue placeholder="Choose who answers" />
          </SelectTrigger>
          <SelectContent>
            {providers.length > 0 ? (
              <SelectGroup>
                <SelectLabel>Providers</SelectLabel>
                {providers.map((option) => (
                  <SelectItem
                    key={option.key}
                    value={option.key}
                    disabled={!option.usable}
                  >
                    {option.label}
                    <span className="text-xs text-muted-foreground">
                      {REACH[option.reach].label}
                    </span>
                  </SelectItem>
                ))}
              </SelectGroup>
            ) : null}
            {agents.length > 0 ? (
              <SelectGroup>
                <SelectLabel>External agents</SelectLabel>
                {agents.map((option) => (
                  <SelectItem
                    key={option.key}
                    value={option.key}
                    disabled={!option.usable}
                  >
                    {option.usable || !option.reason ? (
                      option.label
                    ) : (
                      // Why it cannot be chosen, where it cannot be chosen:
                      // a greyed name alone reads as a bug.
                      <span className="flex max-w-72 flex-col gap-0.5">
                        <span>{option.label}</span>
                        <span
                          data-slot="destination-refused"
                          className="text-xs whitespace-normal text-muted-foreground"
                        >
                          {option.reason}
                        </span>
                      </span>
                    )}
                  </SelectItem>
                ))}
              </SelectGroup>
            ) : null}
          </SelectContent>
        </Select>

        {selected?.kind === "provider" && model ? (
          <Select
            value={model}
            items={modelIds.map((id) => ({ value: id, label: id }))}
            disabled={running}
            onOpenChange={(open) => {
              if (open) onModelsWanted?.()
            }}
            onValueChange={(value) => {
              if (typeof value === "string") onSelectModel(value)
            }}
          >
            <SelectTrigger
              size="sm"
              aria-label="Model"
              className="min-w-36 font-mono"
            >
              <SelectValue />
            </SelectTrigger>
            <SelectContent>
              <SelectGroup>
                {modelIds.map((id) => (
                  <SelectItem key={id} value={id} className="font-mono">
                    {id}
                  </SelectItem>
                ))}
              </SelectGroup>
            </SelectContent>
          </Select>
        ) : null}

        {modelList?.status === "loading" ? (
          <span
            role="status"
            data-slot="model-list"
            className="inline-flex items-center gap-1 text-xs text-muted-foreground"
          >
            <Spinner aria-hidden role={undefined} className="size-3.5" />
            Listing models…
          </span>
        ) : null}
        {modelList?.status === "error" ? (
          <span
            role="alert"
            data-slot="model-list"
            data-selectable
            className="text-xs text-destructive"
          >
            Models could not be listed: {modelList.message}
          </span>
        ) : null}

        {selected?.kind === "provider" && model && onSelectEffort ? (
          <AssistantReasoningEffort
            efforts={efforts}
            value={effort}
            disabledReason={
              running
                ? "The reasoning effort can be changed once the answer is done."
                : null
            }
            onChange={onSelectEffort}
          />
        ) : null}

        {selected && !selected.usable && selected.reason ? (
          <span className="text-xs text-destructive">{selected.reason}</span>
        ) : null}
        {selected?.kind === "agent" ? (
          <span className="text-xs text-muted-foreground">
            {version ? `${version} · ` : ""}An external agent: Oxyn cannot see
            where it sends your question.
          </span>
        ) : null}
        {selected?.kind === "agent" && selected.unconfined ? (
          <span className="text-xs text-warning" data-slot="agent-unconfined">
            Oxyn cannot confine this agent, so it cannot stop it from running
            commands or editing files on this machine by itself.
          </span>
        ) : null}
      </div>

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
