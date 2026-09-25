import type * as React from "react"
import { HugeiconsIcon } from "@hugeicons/react"
import {
  Alert02Icon,
  InformationCircleIcon,
  TimeQuarterPassIcon,
} from "@hugeicons/core-free-icons"

import { ResultPanel } from "@/components/oxyn/result-panel"
import type { FetchPage } from "@/components/oxyn/result-grid"
import { Alert, AlertDescription, AlertTitle } from "@/components/ui/alert"
import { Button } from "@/components/ui/button"
import {
  Empty,
  EmptyContent,
  EmptyDescription,
  EmptyHeader,
  EmptyMedia,
  EmptyTitle,
} from "@/components/ui/empty"
import { Spinner } from "@/components/ui/spinner"
import type { ResultColumn } from "@/lib/ipc/types"

export type RetainedResultState =
  | { status: "loading" }
  | {
      status: "open"
      result: string
      columns: Array<ResultColumn>
      rows: number
      /** The run ended without a doubt and the stream was exhausted. */
      complete: boolean
      truncated: boolean
    }
  | { status: "expired" }
  | { status: "error"; message: string; retryable: boolean }

/** What these rows are, in the words of the library screen. */
export function retainedNotice(
  state: Extract<RetainedResultState, { status: "open" }>
) {
  if (!state.complete)
    return "Incomplete or uncertain execution. These rows are for inspection; export is unavailable."
  if (state.truncated)
    return "Truncated result. Only retained rows are shown; export is unavailable."
  return "Retained rows only. Opening, scrolling and exporting do not rerun the query."
}

/**
 * A result reopened from history: the rows the backend still retains, read
 * page by page. Nothing here reruns the query — an expired result says so and
 * offers no way to bring the rows back.
 */
export function RetainedResultView({
  state,
  fetchPage,
  onRetry,
  onOpenCopy,
  destination,
  density,
}: {
  state: RetainedResultState
  fetchPage: FetchPage
  /** Only for an error the backend declared retryable. */
  onRetry?: () => void
  /** Opens the statement in a console, unrun. */
  onOpenCopy?: () => void
  /** The connection that console opens on, named before the click. */
  destination: string
  /** The text size control of the result bar. */
  density?: React.ComponentProps<typeof ResultPanel>["density"]
}) {
  switch (state.status) {
    case "loading":
      return (
        <Empty className="h-full border-0" aria-busy>
          <EmptyHeader>
            <EmptyMedia variant="icon">
              <Spinner />
            </EmptyMedia>
            <EmptyTitle>Opening retained rows…</EmptyTitle>
            <EmptyDescription>Nothing is rerun.</EmptyDescription>
          </EmptyHeader>
        </Empty>
      )
    case "expired":
      return (
        <Empty className="h-full border-0">
          <EmptyHeader>
            <EmptyMedia variant="icon">
              <HugeiconsIcon icon={TimeQuarterPassIcon} strokeWidth={2} />
            </EmptyMedia>
            <EmptyTitle>Result unavailable</EmptyTitle>
            <EmptyDescription>
              These rows are no longer retained. No query was rerun.
            </EmptyDescription>
          </EmptyHeader>
          {onOpenCopy ? (
            <EmptyContent>
              <Button size="sm" variant="outline" onClick={onOpenCopy}>
                Open a copy of the statement in <bdi>{destination}</bdi>
              </Button>
            </EmptyContent>
          ) : null}
        </Empty>
      )
    case "error":
      return (
        <div className="p-3">
          <Alert variant="destructive">
            <HugeiconsIcon icon={Alert02Icon} strokeWidth={2} />
            <AlertTitle>Result unavailable. No query was rerun.</AlertTitle>
            <AlertDescription>
              <p dir="auto" className="font-mono break-words">
                {state.message}
              </p>
              {state.retryable && onRetry ? (
                <Button
                  size="sm"
                  variant="outline"
                  className="mt-2"
                  onClick={onRetry}
                >
                  Try again
                </Button>
              ) : null}
            </AlertDescription>
          </Alert>
        </div>
      )
    case "open":
      return (
        <div
          data-slot="retained-result"
          className="flex h-full min-h-0 flex-col"
        >
          <p
            role="note"
            className="flex shrink-0 items-center gap-2 border-b px-3 py-1.5 text-xs text-muted-foreground"
          >
            <HugeiconsIcon
              icon={InformationCircleIcon}
              strokeWidth={2}
              className="size-3.5 shrink-0"
            />
            {retainedNotice(state)}
          </p>
          <div className="min-h-0 flex-1">
            <ResultPanel
              state={{
                status: "populated",
                result: state.result,
                columns: state.columns,
                rows: state.rows,
                complete: state.complete,
                truncated: state.truncated,
                cancelled: false,
              }}
              fetchPage={fetchPage}
              density={density}
            />
          </div>
        </div>
      )
  }
}
