import * as React from "react"
import { HugeiconsIcon } from "@hugeicons/react"
import {
  ArrowDown01Icon,
  DashboardSpeed01Icon,
} from "@hugeicons/core-free-icons"

import {
  CHIP_CHEVRON,
  CHIP_LIST,
  COMPOSER_CHIP,
  chipLabel,
} from "@/components/oxyn/assistant-composer-chip"
import {
  Select,
  SelectContent,
  SelectGroup,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select"
import {
  Tooltip,
  TooltipContent,
  TooltipTrigger,
} from "@/components/ui/tooltip"
import { EFFORT_LABELS } from "@/features/assistant/reasoning-effort"
import type { ReasoningEffort } from "@/lib/ipc/ai"
import { cn } from "@/lib/utils"

/** Said when nothing was chosen: the provider applies its own default. */
const DEFAULT = "Default"

/**
 * How much a built-in provider's model reasons before answering.
 *
 * Only the levels the provider declares for this model, in its order. An
 * empty list renders nothing at all: a model that declares none has no such
 * setting, and a greyed selector would suggest one. External agents have no
 * use for this: their own `thoughtLevel` option plays the part.
 *
 * `null` is « nothing chosen » and reads « Default »: the provider decides,
 * and Oxyn does not claim to know which level that is.
 */
export function AssistantReasoningEffort({
  efforts,
  value,
  disabledReason = null,
  chip = false,
  onChange,
}: {
  /** What the model declares, in order; empty when it declares none. */
  efforts: ReadonlyArray<ReasoningEffort>
  value: ReasoningEffort | null
  /** Why it cannot change now — a question running, for one. */
  disabledReason?: string | null
  /** Drawn as a chip of the composer's bottom bar, a gauge before its level. */
  chip?: boolean
  onChange: (effort: ReasoningEffort) => void
}) {
  const reasonId = React.useId()
  if (efforts.length === 0) return null

  // A value the model no longer declares is not sent (`effortToSend`), so it
  // is not shown as chosen either.
  const current = value !== null && efforts.includes(value) ? value : null
  const label = current === null ? DEFAULT : EFFORT_LABELS[current]
  const disabled = disabledReason !== null

  return (
    <span
      data-slot="assistant-reasoning-effort"
      className="inline-flex min-w-0"
    >
      {/* A disabled control takes no pointer event: the reason sits on a
          wrapper that is always there, so nothing remounts when it appears. */}
      <Tooltip disabled={!disabled}>
        <TooltipTrigger render={<span className="inline-flex min-w-0" />}>
          <Select
            value={current}
            disabled={disabled}
            onValueChange={(next) => {
              if (typeof next === "string" && next !== current) onChange(next)
            }}
          >
            <SelectTrigger
              size="sm"
              aria-label={`Reasoning effort: ${label}`}
              aria-describedby={disabled ? reasonId : undefined}
              title={`Reasoning effort: ${label}`}
              className={cn("max-w-36 min-w-0", chip && COMPOSER_CHIP)}
            >
              {chip ? (
                <HugeiconsIcon
                  icon={DashboardSpeed01Icon}
                  strokeWidth={2}
                  aria-hidden
                />
              ) : null}
              <SelectValue className="min-w-0">
                {() => (
                  <span className={chip ? chipLabel(true) : "truncate"}>
                    {label}
                  </span>
                )}
              </SelectValue>
              {chip ? (
                <HugeiconsIcon
                  icon={ArrowDown01Icon}
                  strokeWidth={2}
                  className={CHIP_CHEVRON}
                  aria-hidden
                />
              ) : null}
            </SelectTrigger>
            <SelectContent
              alignItemWithTrigger={false}
              className={cn(chip && CHIP_LIST)}
            >
              <SelectGroup>
                {efforts.map((effort) => (
                  <SelectItem key={effort} value={effort}>
                    {EFFORT_LABELS[effort]}
                  </SelectItem>
                ))}
              </SelectGroup>
            </SelectContent>
          </Select>
        </TooltipTrigger>
        {disabled ? (
          <TooltipContent className="max-w-72">{disabledReason}</TooltipContent>
        ) : null}
      </Tooltip>
      {disabled ? (
        <span id={reasonId} className="sr-only">
          {disabledReason}
        </span>
      ) : null}
    </span>
  )
}
