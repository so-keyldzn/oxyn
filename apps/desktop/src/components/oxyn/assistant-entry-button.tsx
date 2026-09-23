import * as React from "react"
import { HugeiconsIcon } from "@hugeicons/react"
import { SparklesIcon } from "@hugeicons/core-free-icons"

import { PrivacyTierBadge } from "@/components/oxyn/privacy-tier"
import { Toggle } from "@/components/ui/toggle"
import {
  Tooltip,
  TooltipContent,
  TooltipTrigger,
} from "@/components/ui/tooltip"
import type { AssistantEntry } from "@/features/assistant/availability"
import type { PrivacyTier } from "@/lib/ipc/types"

/**
 * `Ask AI`, in the connection bar, with the connection's tier badge beside it.
 *
 * Three drawings, and the first is the point: `absent` draws nothing — no
 * button and no badge. A greyed control inviting to configure would be an
 * advertisement, not a feature (docs/UX-SPEC.md). `disabled` stays visible
 * and says why — making it vanish would read as a defect (ADR-0006).
 *
 * The badge is drawn here rather than by the bar so that it can never appear
 * without the entry, nor the entry without it: the tier is read before
 * speaking, not after (UX-SPEC « Le niveau se lit avant de parler »). It is
 * the tier of **this** connection (I-04).
 */
export function AssistantEntryButton({
  entry,
  tier = null,
  pressed,
  onPressedChange,
}: {
  entry: AssistantEntry
  /** The current connection's tier; `null` draws the button alone. */
  tier?: PrivacyTier | null
  pressed: boolean
  onPressedChange: (pressed: boolean) => void
}) {
  const reasonId = React.useId()
  if (entry.status === "absent") return null

  const button =
    entry.status === "disabled" ? (
      <>
        <Tooltip>
          <TooltipTrigger
            render={
              <Toggle
                variant="outline"
                size="sm"
                pressed={pressed}
                // Focusable, so the reason can be read; pressing opens the
                // panel, which explains too and offers no way to send.
                aria-describedby={reasonId}
                onPressedChange={onPressedChange}
                className="text-muted-foreground"
              />
            }
          >
            <HugeiconsIcon
              icon={SparklesIcon}
              strokeWidth={2}
              data-icon="inline-start"
            />
            Ask AI
          </TooltipTrigger>
          <TooltipContent className="max-w-72">{entry.reason}</TooltipContent>
        </Tooltip>
        {/* Outside the button: inside, it would join its name and be read a
            second time as its description. */}
        <span id={reasonId} className="sr-only">
          Unavailable: {entry.reason}
        </span>
      </>
    ) : (
      <Toggle
        variant="outline"
        size="sm"
        pressed={pressed}
        onPressedChange={onPressedChange}
        aria-label="Ask AI"
      >
        <HugeiconsIcon
          icon={SparklesIcon}
          strokeWidth={2}
          data-icon="inline-start"
        />
        Ask AI
      </Toggle>
    )

  return (
    <div
      data-slot="assistant-entry"
      className="flex shrink-0 items-center gap-2"
    >
      {tier ? <PrivacyTierBadge tier={tier} /> : null}
      {button}
    </div>
  )
}
