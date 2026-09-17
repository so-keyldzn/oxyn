import * as React from "react"
import { HugeiconsIcon } from "@hugeicons/react"
import { Alert02Icon } from "@hugeicons/core-free-icons"

import { Alert, AlertDescription, AlertTitle } from "@/components/ui/alert"
import { Button } from "@/components/ui/button"

export interface BackendFailure {
  message: string
  /** From the backend, never guessed from the message (I-13). */
  retryable: boolean
}

/**
 * An error as a professional reads it (docs/UX-SPEC.md « Les erreurs
 * s'adressent à un professionnel »): the server's own words, whether trying
 * again can help, and what to do next.
 *
 * `onRetry` is offered only for a transient error, and only as the user's
 * click: nothing is retried on its own (front.md « Pas de retry »).
 */
export function BackendErrorAlert({
  title,
  error,
  context,
  onRetry,
  retryLabel = "Try again",
  nextStep,
  children,
}: {
  title: string
  error: BackendFailure
  /** What the server does not say: which connection, which action. */
  context?: React.ReactNode
  onRetry?: () => void
  retryLabel?: string
  /** The next step when trying again as is would fail the same way. */
  nextStep?: string
  children?: React.ReactNode
}) {
  return (
    <Alert variant="destructive" data-slot="backend-error">
      <HugeiconsIcon icon={Alert02Icon} strokeWidth={2} />
      <AlertTitle>{title}</AlertTitle>
      <AlertDescription className="flex min-w-0 flex-col items-start gap-2">
        {context ? (
          <p className="text-xs text-muted-foreground">{context}</p>
        ) : null}
        {/* `wrap-anywhere`, not `break-words`: only the former lowers the
            min-content width, and the Alert is a grid whose text column grows
            to it — a 300-character relation name widened the whole window. */}
        <pre
          data-selectable
          dir="auto"
          className="max-h-40 w-full overflow-auto font-mono text-xs wrap-anywhere whitespace-pre-wrap text-foreground"
        >
          {error.message}
        </pre>
        <p className="text-xs text-muted-foreground">
          {error.retryable
            ? "This error is transient: trying again may succeed."
            : (nextStep ?? "Trying again as is will fail the same way.")}
        </p>
        {error.retryable && onRetry ? (
          <Button variant="outline" size="sm" onClick={onRetry}>
            {retryLabel}
          </Button>
        ) : null}
        {children}
      </AlertDescription>
    </Alert>
  )
}
