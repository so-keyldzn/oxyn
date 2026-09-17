import { HugeiconsIcon } from "@hugeicons/react"
import { Cancel01Icon, Clock01Icon, SentIcon } from "@hugeicons/core-free-icons"

import { Button } from "@/components/ui/button"
import type { QueuedMessage } from "@/features/assistant/thread"

/**
 * Messages typed while the assistant was answering.
 *
 * They leave when the run ends — one at a time, as follow-ups. A queued
 * message never answers a pending approval: only the review does. After a
 * failure they wait for the user, who reads the failure first.
 */
export function AssistantQueue({
  queue,
  held,
  onSendNow,
  onRemove,
}: {
  queue: ReadonlyArray<QueuedMessage>
  /** The last run failed: nothing leaves on its own. */
  held: boolean
  onSendNow: (key: string) => void
  onRemove: (key: string) => void
}) {
  if (queue.length === 0) return null
  return (
    <section
      data-slot="assistant-queue"
      aria-label={`${queue.length} queued message(s)`}
      className="flex flex-col gap-1.5"
    >
      <p className="flex items-center gap-1.5 text-xs text-muted-foreground">
        <HugeiconsIcon
          icon={Clock01Icon}
          strokeWidth={2}
          className="size-3.5"
          aria-hidden
        />
        {held
          ? "Queued · the last answer failed, so these wait for you."
          : "Queued · sent when the current answer ends. They approve nothing."}
      </p>
      <ol className="flex flex-col gap-1">
        {queue.map((message, index) => (
          <li
            key={message.key}
            className="flex min-w-0 items-center gap-1 rounded-md border border-dashed py-1 pr-1 pl-2 text-sm"
          >
            <span className="w-4 shrink-0 text-xs text-muted-foreground tabular-nums">
              {index + 1}
            </span>
            <span
              className="min-w-0 flex-1 truncate"
              dir="auto"
              title={message.text}
            >
              {message.text}
            </span>
            {held ? (
              <Button
                size="icon-xs"
                variant="ghost"
                aria-label={`Send queued message ${index + 1} now`}
                onClick={() => onSendNow(message.key)}
              >
                <HugeiconsIcon icon={SentIcon} strokeWidth={2} />
              </Button>
            ) : null}
            <Button
              size="icon-xs"
              variant="ghost"
              aria-label={`Remove queued message ${index + 1}`}
              onClick={() => onRemove(message.key)}
            >
              <HugeiconsIcon icon={Cancel01Icon} strokeWidth={2} />
            </Button>
          </li>
        ))}
      </ol>
    </section>
  )
}
