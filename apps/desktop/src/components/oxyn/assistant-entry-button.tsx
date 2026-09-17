import * as React from "react"
import { HugeiconsIcon } from "@hugeicons/react"
import { AiMagicIcon } from "@hugeicons/core-free-icons"

import { Toggle } from "@/components/ui/toggle"
import {
  Tooltip,
  TooltipContent,
  TooltipTrigger,
} from "@/components/ui/tooltip"
import type { AssistantEntry } from "@/features/assistant/availability"

/**
 * `Ask AI`, in the connection bar next to the tier badge.
 *
 * Three drawings, and the first is the point: `absent` draws nothing. A
 * greyed control inviting to configure would be an advertisement, not a
 * feature (docs/UX-SPEC.md). `disabled` stays visible and says why — making
 * it vanish would read as a defect (ADR-0006).
 */
export function AssistantEntryButton({
  entry,
  pressed,
  onPressedChange,
}: {
  entry: AssistantEntry
  pressed: boolean
  onPressedChange: (pressed: boolean) => void
}) {
  const reasonId = React.useId()
  if (entry.status === "absent") return null

  if (entry.status === "disabled") {
    return (
      <Tooltip>
        <TooltipTrigger
          render={
            <Toggle
              variant="outline"
              size="sm"
              pressed={pressed}
              // Focusable, so the reason can be read; pressing opens the panel,
              // which explains too and offers no way to send.
              aria-describedby={reasonId}
              onPressedChange={onPressedChange}
              className="text-muted-foreground"
            />
          }
        >
          <HugeiconsIcon
            icon={AiMagicIcon}
            strokeWidth={2}
            data-icon="inline-start"
          />
          Ask AI
          <span id={reasonId} className="sr-only">
            Unavailable: {entry.reason}
          </span>
        </TooltipTrigger>
        <TooltipContent className="max-w-72">{entry.reason}</TooltipContent>
      </Tooltip>
    )
  }

  return (
    <Toggle
      variant="outline"
      size="sm"
      pressed={pressed}
      onPressedChange={onPressedChange}
      aria-label="Ask AI"
    >
      <HugeiconsIcon
        icon={AiMagicIcon}
        strokeWidth={2}
        data-icon="inline-start"
      />
      Ask AI
    </Toggle>
  )
}
