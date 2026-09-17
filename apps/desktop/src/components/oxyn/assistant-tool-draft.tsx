import { HugeiconsIcon } from "@hugeicons/react"
import { CommandLineIcon } from "@hugeicons/core-free-icons"

import { Spinner } from "@/components/ui/spinner"
import type { ToolDraftEntry } from "@/features/assistant/transcript"

/**
 * A tool call the model is still writing.
 *
 * Nothing is translated, nothing is submitted: the arguments are partial JSON
 * as they stream, shown so a long call does not look like a stall.
 */
export function AssistantToolDraft({ entry }: { entry: ToolDraftEntry }) {
  return (
    <section
      data-slot="assistant-tool-draft"
      aria-label={`The model is writing a call to ${entry.tool}`}
      className="flex min-w-0 flex-col overflow-hidden rounded-lg border border-dashed bg-card/50 text-sm"
    >
      <header className="flex items-center gap-2 px-3 py-2 text-xs text-muted-foreground">
        <HugeiconsIcon
          icon={CommandLineIcon}
          strokeWidth={2}
          className="size-4"
          aria-hidden
        />
        <span className="font-mono text-foreground">{entry.tool}</span>
        <span>· writing the call, nothing has run</span>
        <Spinner className="ml-auto" />
      </header>
      {entry.arguments ? (
        <pre
          data-selectable
          tabIndex={0}
          aria-label="Arguments being written"
          className="max-h-40 overflow-auto border-t px-3 py-2 font-mono text-xs leading-5 break-all whitespace-pre-wrap text-muted-foreground outline-none focus-visible:ring-2 focus-visible:ring-ring"
        >
          {entry.arguments}
        </pre>
      ) : null}
    </section>
  )
}
