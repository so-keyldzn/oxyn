import * as React from "react"

import {
  Select,
  SelectContent,
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
  onChange,
}: {
  /** What the model declares, in order; empty when it declares none. */
  efforts: ReadonlyArray<ReasoningEffort>
  value: ReasoningEffort | null
  /** Why it cannot change now — a question running, for one. */
  disabledReason?: string | null
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
              className="max-w-36 min-w-0"
            >
              <SelectValue className="min-w-0">
                {() => <span className="truncate">{label}</span>}
              </SelectValue>
            </SelectTrigger>
            <SelectContent alignItemWithTrigger={false}>
              {efforts.map((effort) => (
                <SelectItem key={effort} value={effort}>
                  {EFFORT_LABELS[effort]}
                </SelectItem>
              ))}
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
