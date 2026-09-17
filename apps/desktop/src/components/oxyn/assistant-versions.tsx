import { HugeiconsIcon } from "@hugeicons/react"
import { ArrowLeft01Icon, ArrowRight01Icon } from "@hugeicons/core-free-icons"

import { Button } from "@/components/ui/button"
import type { Versions } from "@/features/assistant/thread"

/** ‹ 2 / 3 ›: the versions of one exchange. Hidden when there is one. */
export function AssistantVersions({
  versions,
  disabled = false,
  onSelect,
}: {
  versions: Versions
  disabled?: boolean
  onSelect: (node: number) => void
}) {
  if (versions.count < 2) return null
  return (
    <div
      data-slot="assistant-versions"
      role="group"
      aria-label="Versions of this exchange"
      className="inline-flex items-center text-xs text-muted-foreground"
    >
      <Button
        size="icon-xs"
        variant="ghost"
        aria-label="Previous version"
        disabled={disabled || versions.previous === null}
        onClick={() => {
          if (versions.previous !== null) onSelect(versions.previous)
        }}
      >
        <HugeiconsIcon icon={ArrowLeft01Icon} strokeWidth={2} />
      </Button>
      <span className="min-w-8 text-center tabular-nums" aria-live="polite">
        {versions.position} / {versions.count}
      </span>
      <Button
        size="icon-xs"
        variant="ghost"
        aria-label="Next version"
        disabled={disabled || versions.next === null}
        onClick={() => {
          if (versions.next !== null) onSelect(versions.next)
        }}
      >
        <HugeiconsIcon icon={ArrowRight01Icon} strokeWidth={2} />
      </Button>
    </div>
  )
}
