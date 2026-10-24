import { HugeiconsIcon } from "@hugeicons/react"
import {
  ArrowDown01Icon,
  CloudIcon,
  ComputerIcon,
  HelpCircleIcon,
} from "@hugeicons/core-free-icons"

import {
  CHIP_CHEVRON,
  CHIP_LIST,
  COMPOSER_CHIP,
  chipLabel,
} from "@/components/oxyn/assistant-composer-chip"
import { AssistantReasoningEffort } from "@/components/oxyn/assistant-reasoning-effort"
import { ProviderLogo } from "@/components/oxyn/provider-logo"
import { Spinner } from "@/components/ui/spinner"
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
import type { ModelChoice, ProviderReach, ReasoningEffort } from "@/lib/ipc/ai"
import { cn } from "@/lib/utils"

export const REACH: Record<
  ProviderReach,
  { label: string; icon: typeof CloudIcon }
> = {
  local: { label: "This machine", icon: ComputerIcon },
  remote: { label: "Cloud", icon: CloudIcon },
  // Counts as remote; said as what it is.
  unresolved: { label: "Unresolved", icon: HelpCircleIcon },
}

/** The list of models of the selected provider, while it is not there. */
export type ModelListState =
  { status: "loading" } | { status: "error"; message: string }

/** The mark of whoever answers: its brand's, or the icon of its kind. */
export function DestinationMark({
  option,
  className,
}: {
  option: Pick<DestinationOption, "kind" | "brand">
  className?: string
}) {
  return (
    <ProviderLogo
      brand={option.brand ?? ""}
      fallback={option.kind === "agent" ? "agent" : "endpoint"}
      className={cn("size-3.5 shrink-0", className)}
    />
  )
}

/** A destination's name in the list, with why it cannot be chosen. */
function OptionContent({
  option,
  where,
}: {
  option: DestinationOption
  where: string | null
}) {
  // Bounded here: the list's item text does not shrink, and a long name
  // would draw past the list's edge.
  if (option.usable || !option.reason)
    return (
      <span className="flex max-w-72 min-w-0 items-center gap-2">
        <DestinationMark option={option} />
        <span className="min-w-0 truncate" title={option.label}>
          {option.label}
        </span>
        {where ? (
          <span className="shrink-0 text-xs text-muted-foreground">
            {where}
          </span>
        ) : null}
      </span>
    )
  // A greyed name alone reads as a bug.
  return (
    <span className="flex max-w-72 min-w-0 items-start gap-2">
      <DestinationMark option={option} className="mt-0.5" />
      <span className="flex min-w-0 flex-col gap-0.5">
        <span className="truncate" title={option.label}>
          {option.label}
        </span>
        <span
          data-slot="destination-refused"
          className="text-xs wrap-anywhere whitespace-normal text-muted-foreground"
        >
          {option.reason}
        </span>
      </span>
    </span>
  )
}

/**
 * Who answers, its model and how hard it reasons: the chips of the composer's
 * bottom bar that govern the next question (docs/UX-SPEC.md, « Who answers is
 * chosen in the panel »).
 *
 * A provider reads as its mark then its model, two selectors side by side; an
 * agent as its mark and its name, its own settings beside it. All are inert
 * while an answer is being written.
 */
export function AssistantDestinationChips({
  destinations,
  selected,
  model,
  models,
  running,
  onSelectDestination,
  onSelectModel,
  onModelsWanted,
  efforts = [],
  effort = null,
  onSelectEffort,
}: {
  destinations: ReadonlyArray<DestinationOption>
  selected: DestinationOption | null
  /** The model for the selected provider; `null` for an agent. */
  model: string | null
  /** What the provider listed, when asked; `null` before. */
  models: ReadonlyArray<ModelChoice> | null
  running: boolean
  onSelectDestination: (key: string) => void
  onSelectModel: (model: string) => void
  /** Opening the model list; the list itself is asked for by the caller. */
  onModelsWanted?: () => void
  /** The efforts the selected model declares; empty hides the selector. */
  efforts?: ReadonlyArray<ReasoningEffort>
  /** The effort chosen for this provider; `null` leaves the provider's default. */
  effort?: ReasoningEffort | null
  onSelectEffort?: (effort: ReasoningEffort) => void
}) {
  const providers = destinations.filter((option) => option.kind === "provider")
  const agents = destinations.filter((option) => option.kind === "agent")
  const modelIds = Array.from(
    new Set([
      ...(selected?.model ? [selected.model] : []),
      ...(model ? [model] : []),
      ...(models ?? []).map((choice) => choice.id),
    ])
  )
  const withModel = selected?.kind === "provider" && model !== null

  return (
    <span
      data-slot="assistant-destination-chips"
      className="inline-flex min-w-0 items-center"
    >
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
          // With a model beside it, the mark alone: the model is what the
          // provider's chip reads as, and the name is said in the header.
          title={selected ? `Who answers: ${selected.label}` : undefined}
          className={cn(COMPOSER_CHIP, "max-w-48", withModel && "pr-0.5")}
        >
          <SelectValue className="min-w-0">
            {() =>
              selected ? (
                <>
                  <DestinationMark option={selected} />
                  {withModel ? null : (
                    <span className={chipLabel()}>{selected.label}</span>
                  )}
                </>
              ) : (
                <span className="text-muted-foreground">
                  Choose who answers
                </span>
              )
            }
          </SelectValue>
          {withModel ? null : (
            <HugeiconsIcon
              icon={ArrowDown01Icon}
              strokeWidth={2}
              className={CHIP_CHEVRON}
              aria-hidden
            />
          )}
        </SelectTrigger>
        <SelectContent alignItemWithTrigger={false} className={CHIP_LIST}>
          {providers.length > 0 ? (
            <SelectGroup>
              <SelectLabel>Providers</SelectLabel>
              {providers.map((option) => (
                <SelectItem
                  key={option.key}
                  value={option.key}
                  disabled={!option.usable}
                >
                  <OptionContent
                    option={option}
                    where={REACH[option.reach].label}
                  />
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
                  <OptionContent option={option} where={null} />
                </SelectItem>
              ))}
            </SelectGroup>
          ) : null}
        </SelectContent>
      </Select>

      {withModel ? (
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
            title={`Model: ${model}`}
            className={cn(COMPOSER_CHIP, "max-w-56 pl-0.5")}
          >
            <SelectValue className="min-w-0">
              {() => <span className={chipLabel()}>{model}</span>}
            </SelectValue>
            <HugeiconsIcon
              icon={ArrowDown01Icon}
              strokeWidth={2}
              className={CHIP_CHEVRON}
              aria-hidden
            />
          </SelectTrigger>
          <SelectContent alignItemWithTrigger={false} className={CHIP_LIST}>
            <SelectGroup>
              {modelIds.map((id) => (
                <SelectItem key={id} value={id} className="font-mono">
                  <span className="block max-w-72 truncate" title={id}>
                    {id}
                  </span>
                </SelectItem>
              ))}
            </SelectGroup>
          </SelectContent>
        </Select>
      ) : null}

      {withModel && onSelectEffort ? (
        <AssistantReasoningEffort
          chip
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
    </span>
  )
}

/**
 * The model list while it loads or after it failed, in the endpoint's own
 * words: the audience reads error messages.
 */
export function AssistantModelListNote({
  modelList,
}: {
  modelList: ModelListState | null
}) {
  if (modelList?.status === "loading")
    return (
      <span
        role="status"
        data-slot="model-list"
        className="inline-flex items-center gap-1 text-xs text-muted-foreground"
      >
        <Spinner aria-hidden role={undefined} className="size-3.5" />
        Listing models…
      </span>
    )
  if (modelList?.status === "error")
    return (
      <span
        role="alert"
        data-slot="model-list"
        data-selectable
        className="text-xs text-destructive"
      >
        Models could not be listed: {modelList.message}
      </span>
    )
  return null
}
