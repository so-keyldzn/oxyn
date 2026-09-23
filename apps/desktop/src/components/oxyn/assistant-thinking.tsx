import * as React from "react"
import { HugeiconsIcon } from "@hugeicons/react"
import { AiBrain01Icon, ArrowDown01Icon } from "@hugeicons/core-free-icons"

import {
  Collapsible,
  CollapsibleContent,
  CollapsibleTrigger,
} from "@/components/ui/collapsible"
import type { ThinkingEntry } from "@/features/assistant/transcript"
import { cn } from "@/lib/utils"

function seconds(ms: number) {
  const value = ms / 1000
  return value < 10 ? value.toFixed(1) : Math.round(value).toString()
}

/** The one line a reasoning block shows folded. */
export function thinkingSummary(entry: ThinkingEntry) {
  if (entry.elapsedMs === null) return "Thinking…"
  const duration = `Thought for ${seconds(entry.elapsedMs)} s`
  if (entry.redacted && entry.text.trim() === "") {
    return `${duration} · hidden by the provider`
  }
  return duration
}

/**
 * The model's reasoning, apart from its answer and folded by default.
 *
 * It is a draft, not an opinion: it is shown so the wait is explained, and so
 * a reader can check how an answer came about. When the provider hides it, the
 * block says so rather than pretending there was none.
 */
/** The line shown when there is nothing to unfold. */
function Summary({ entry }: { entry: ThinkingEntry }) {
  const streaming = entry.elapsedMs === null
  return (
    <p
      data-slot="assistant-thinking"
      className="inline-flex w-fit items-center gap-1.5 py-0.5 text-xs text-muted-foreground"
    >
      <HugeiconsIcon
        icon={AiBrain01Icon}
        strokeWidth={2}
        className="size-3.5"
        aria-hidden
      />
      <span
        className={cn(
          "tabular-nums",
          streaming && "shimmer motion-reduce:shimmer-none"
        )}
      >
        {thinkingSummary(entry)}
      </span>
    </p>
  )
}

export function AssistantThinking({ entry }: { entry: ThinkingEntry }) {
  const [open, setOpen] = React.useState(false)
  const streaming = entry.elapsedMs === null
  const visible = entry.text.trim() !== ""
  // Nothing to unfold renders no control at all: a dead button reads as
  // something that should work.
  if (!visible) return <Summary entry={entry} />

  return (
    <Collapsible
      data-slot="assistant-thinking"
      open={open}
      onOpenChange={setOpen}
      className="flex min-w-0 flex-col text-sm"
    >
      <CollapsibleTrigger className="group/thinking inline-flex w-fit items-center gap-1.5 rounded-md py-0.5 pr-1 text-xs text-muted-foreground outline-none hover:text-foreground focus-visible:ring-2 focus-visible:ring-ring">
        <HugeiconsIcon
          icon={AiBrain01Icon}
          strokeWidth={2}
          className="size-3.5"
          aria-hidden
        />
        <span
          className={cn(
            "tabular-nums",
            streaming && "shimmer motion-reduce:shimmer-none"
          )}
          aria-live={streaming ? "off" : "polite"}
        >
          {thinkingSummary(entry)}
        </span>
        <HugeiconsIcon
          icon={ArrowDown01Icon}
          strokeWidth={2}
          className="size-3.5 transition-transform group-data-[panel-open]/thinking:rotate-180 motion-reduce:transition-none"
          aria-hidden
        />
      </CollapsibleTrigger>
      <CollapsibleContent className="overflow-hidden">
        <div
          data-selectable
          tabIndex={0}
          role="region"
          aria-label="Model reasoning"
          className="mt-1.5 max-h-72 overflow-auto border-l-2 pl-3 text-xs leading-5 whitespace-pre-wrap text-muted-foreground outline-none focus-visible:ring-2 focus-visible:ring-ring"
        >
          {entry.text}
          {entry.redacted ? (
            <p className="mt-2 italic">
              Part of this reasoning was hidden by the provider.
            </p>
          ) : null}
        </div>
      </CollapsibleContent>
    </Collapsible>
  )
}
