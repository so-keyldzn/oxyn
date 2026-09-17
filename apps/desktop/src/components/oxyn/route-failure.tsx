import * as React from "react"
import { HugeiconsIcon } from "@hugeicons/react"
import {
  Alert02Icon,
  ArrowLeft01Icon,
  RefreshIcon,
  Search01Icon,
} from "@hugeicons/core-free-icons"

import { Button } from "@/components/ui/button"
import {
  Empty,
  EmptyContent,
  EmptyDescription,
  EmptyHeader,
  EmptyMedia,
  EmptyTitle,
} from "@/components/ui/empty"

/**
 * What a screen shows when it failed to render, or does not exist.
 *
 * Says what happened, keeps the technical message readable for a data
 * professional, and offers the two ways out that always work: reload the
 * window, or go back to the connections. Neither closes a session nor
 * discards a draft: drafts are written as they are typed (ADR-0024).
 */
export function RouteFailure({
  kind,
  message = null,
  onReload,
  onBack,
}: {
  kind: "error" | "notFound"
  /** The error's message. Never a stack, never a bound value. */
  message?: string | null
  onReload?: () => void
  onBack: () => void
}) {
  const titleRef = React.useRef<HTMLHeadingElement>(null)
  // Focus lands on the explanation, not behind a screen that is gone.
  React.useEffect(() => titleRef.current?.focus(), [])

  return (
    <main
      data-slot="route-failure"
      className="flex h-full items-center justify-center bg-background p-6"
    >
      <Empty className="max-w-lg border-0">
        <EmptyHeader>
          <EmptyMedia variant="icon">
            <HugeiconsIcon
              icon={kind === "error" ? Alert02Icon : Search01Icon}
              strokeWidth={2}
            />
          </EmptyMedia>
          <EmptyTitle>
            <h1 ref={titleRef} tabIndex={-1} className="outline-none">
              {kind === "error"
                ? "This screen stopped working"
                : "This screen does not exist"}
            </h1>
          </EmptyTitle>
          <EmptyDescription>
            {kind === "error"
              ? "Your drafts are saved locally and your connections were left as they were."
              : "The address does not match any screen of Oxyn."}
          </EmptyDescription>
        </EmptyHeader>
        {message ? (
          <pre
            // Scrollable, so reachable by keyboard.
            tabIndex={0}
            aria-label="Error message"
            dir="auto"
            className="max-h-40 w-full overflow-auto rounded-md border bg-muted px-3 py-2 text-left font-mono text-xs break-words whitespace-pre-wrap text-muted-foreground"
          >
            {message}
          </pre>
        ) : null}
        <EmptyContent className="flex-row justify-center gap-2">
          {onReload ? (
            <Button size="sm" onClick={onReload}>
              <HugeiconsIcon
                icon={RefreshIcon}
                strokeWidth={2}
                data-icon="inline-start"
              />
              Reload
            </Button>
          ) : null}
          <Button
            size="sm"
            variant={onReload ? "outline" : "default"}
            onClick={onBack}
          >
            <HugeiconsIcon
              icon={ArrowLeft01Icon}
              strokeWidth={2}
              data-icon="inline-start"
            />
            Back to connections
          </Button>
        </EmptyContent>
      </Empty>
    </main>
  )
}
