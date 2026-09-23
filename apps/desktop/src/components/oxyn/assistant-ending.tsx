import { HugeiconsIcon } from "@hugeicons/react"
import { PlayIcon } from "@hugeicons/core-free-icons"

import { Button } from "@/components/ui/button"
import { Marker, MarkerContent } from "@/components/ui/marker"
import { endingLine } from "@/features/assistant/transcript"
import type { Ending } from "@/lib/ipc/ai"
import { cn } from "@/lib/utils"

const AWAITING_REVIEW =
  "Waiting for your review: a command the agent submitted has not run."

/** Endings that ask for the reader's attention. */
function notable(ending: Ending) {
  return (
    (ending.type === "answered" && ending.truncated) ||
    ending.type === "paused" ||
    ending.type === "refused" ||
    ending.type === "turnLimit" ||
    ending.type === "agentLimit"
  )
}

/**
 * How a run ended, on its own line.
 *
 * A cut answer, a paused one and a refusal each say what they are: a cut
 * answer that looks finished reads as a wrong one. « Continue » is offered
 * where going on makes sense, and only as a click — a paused turn resumes at
 * the user's cost, not on its own.
 *
 * While a command of the exchange still waits for approval, the line says so
 * instead of « Answered »: the work is not done until the user decides.
 *
 * Not a live region: the panel announces its state once, in one place.
 */
export function AssistantEnding({
  ending,
  awaitingReview = false,
  onContinue,
  continueDisabled = false,
}: {
  ending: Ending
  awaitingReview?: boolean
  onContinue?: () => void
  continueDisabled?: boolean
}) {
  const attention = awaitingReview || notable(ending)
  return (
    <div
      data-slot="assistant-ending"
      data-ending={awaitingReview ? "awaitingReview" : ending.type}
      className="flex flex-wrap items-center gap-2"
    >
      <Marker
        variant="border"
        className={cn(
          "min-w-0 flex-1 text-xs tabular-nums",
          attention && "text-warning"
        )}
      >
        <MarkerContent>
          {awaitingReview ? AWAITING_REVIEW : endingLine(ending)}
        </MarkerContent>
      </Marker>
      {onContinue ? (
        <Button
          size="xs"
          variant="outline"
          disabled={continueDisabled}
          onClick={onContinue}
        >
          <HugeiconsIcon
            icon={PlayIcon}
            strokeWidth={2}
            data-icon="inline-start"
          />
          Continue
        </Button>
      ) : null}
    </div>
  )
}
