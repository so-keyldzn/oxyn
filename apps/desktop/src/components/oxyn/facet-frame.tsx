import type * as React from "react"
import { HugeiconsIcon } from "@hugeicons/react"
import {
  Alert02Icon,
  ArrowReloadHorizontalIcon,
  CancelCircleIcon,
  DatabaseIcon,
} from "@hugeicons/core-free-icons"

import { Alert, AlertDescription, AlertTitle } from "@/components/ui/alert"
import { Badge } from "@/components/ui/badge"
import { Button } from "@/components/ui/button"
import {
  Empty,
  EmptyContent,
  EmptyDescription,
  EmptyHeader,
  EmptyMedia,
  EmptyTitle,
} from "@/components/ui/empty"
import { Skeleton } from "@/components/ui/skeleton"
import { Spinner } from "@/components/ui/spinner"
import type { FacetFreshness } from "@/lib/ipc/metadata"

/** Where the read of one facet stands, as the front tracks it. */
export type FacetLoad =
  | { status: "idle" }
  | { status: "loading" }
  | { status: "cancelled" }
  | { status: "error"; message: string }

export const IDLE: FacetLoad = { status: "idle" }

/** How fresh a facet is, in words. Pure, so it is tested without a DOM. */
export function freshnessLabel(freshness: FacetFreshness): string {
  switch (freshness.state) {
    case "never":
      return "Not loaded"
    case "invalidated":
      return "Stale"
    case "fetched": {
      const date = new Date(freshness.fetchedAt)
      return Number.isNaN(date.getTime())
        ? "Loaded"
        : `Loaded ${date.toLocaleTimeString("en-US", { hour: "2-digit", minute: "2-digit" })}`
    }
  }
}

/**
 * The frame every metadata tab shares: the five states of a remote view, and
 * how fresh what is shown is (docs/UX-SPEC.md, « États d'une vue »).
 *
 * « Stale » is said, not hidden: after a DDL sent from Oxyn the cache is
 * invalidated, and what is on screen is the read from before (ADR-0022). A
 * failed refresh keeps the previous read on screen and says it may be
 * outdated, as the GPUI definition panel does; it never retries on its own.
 */
export function FacetFrame({
  label,
  freshness,
  load,
  unsupported,
  hasValue,
  empty,
  emptyText,
  onRefresh,
  onCancel,
  actions,
  children,
}: {
  /** What is loaded, lower case: « constraints », « the definition ». */
  label: string
  freshness: FacetFreshness
  load: FacetLoad
  /** Set when the session cannot load this facet (ADR-0003). */
  unsupported: string | null
  hasValue: boolean
  empty: boolean
  emptyText: string
  onRefresh: () => void
  onCancel: () => void
  /** Extra header actions, shown once there is something to act on. */
  actions?: React.ReactNode
  children: React.ReactNode
}) {
  if (unsupported) {
    return (
      <Empty className="h-full border-0">
        <EmptyHeader>
          <EmptyMedia variant="icon">
            <HugeiconsIcon icon={DatabaseIcon} strokeWidth={2} />
          </EmptyMedia>
          <EmptyTitle>Not available for this session</EmptyTitle>
          <EmptyDescription>{unsupported}</EmptyDescription>
        </EmptyHeader>
      </Empty>
    )
  }

  const loading = load.status === "loading"
  const stale = freshness.state === "invalidated"

  return (
    <div className="flex h-full min-h-0 flex-col">
      <div className="flex h-9 shrink-0 items-center gap-2 border-b px-3 text-xs">
        <Badge variant={stale ? "outline" : "secondary"}>
          {freshnessLabel(freshness)}
        </Badge>
        {stale ? (
          <span
            title="A DDL changed this connection since this was read."
            className="truncate text-muted-foreground"
          >
            A DDL changed this connection since this was read.
          </span>
        ) : null}
        {/* Refresh and Cancel keep their own place, as Run and Stop do
            (docs/UX-SPEC.md, « Portée de Run »): one button whose meaning
            flips is a missed click away from restarting what was to be
            stopped. The one with nothing to do is dimmed and inert. */}
        <div className="ml-auto flex shrink-0 items-center gap-2">
          {hasValue ? actions : null}
          <Button
            size="sm"
            variant="outline"
            disabled={loading}
            onClick={onRefresh}
          >
            <HugeiconsIcon
              icon={ArrowReloadHorizontalIcon}
              strokeWidth={2}
              data-icon="inline-start"
            />
            Refresh
          </Button>
          <Button
            size="sm"
            variant="outline"
            disabled={!loading}
            onClick={onCancel}
          >
            <HugeiconsIcon
              icon={CancelCircleIcon}
              strokeWidth={2}
              data-icon="inline-start"
            />
            Cancel
          </Button>
        </div>
      </div>

      {load.status === "error" ? (
        <Alert
          variant="destructive"
          className="rounded-none border-x-0 border-t-0"
        >
          <HugeiconsIcon icon={Alert02Icon} strokeWidth={2} />
          <AlertTitle>Loading {label} failed</AlertTitle>
          <AlertDescription>
            <pre
              data-selectable
              // A long identifier must not widen the Alert's grid column.
              className="font-mono text-xs wrap-anywhere whitespace-pre-wrap"
            >
              {load.message}
            </pre>
            {hasValue ? (
              <p className="mt-1 text-xs">
                Showing the last successful read; it may be outdated.
              </p>
            ) : null}
          </AlertDescription>
        </Alert>
      ) : null}

      {load.status === "cancelled" ? (
        <p
          role="status"
          className="border-b px-3 py-1.5 text-xs text-muted-foreground"
        >
          Loading cancelled. What was loaded before is kept; Refresh loads
          again.
        </p>
      ) : null}

      <div className="min-h-0 flex-1 overflow-auto">
        {hasValue ? (
          empty ? (
            <Empty className="h-full border-0">
              <EmptyHeader>
                <EmptyTitle>Nothing reported</EmptyTitle>
                <EmptyDescription>{emptyText}</EmptyDescription>
              </EmptyHeader>
            </Empty>
          ) : (
            children
          )
        ) : loading ? (
          <div
            role="status"
            aria-label={`Loading ${label}`}
            className="flex flex-col gap-2 p-4"
          >
            <span className="flex items-center gap-2 text-xs text-muted-foreground">
              <Spinner /> Loading {label}…
            </span>
            {Array.from({ length: 4 }, (_, index) => (
              <Skeleton key={index} className="h-6 w-full" />
            ))}
          </div>
        ) : load.status === "error" ? null : (
          <Empty className="h-full border-0">
            <EmptyHeader>
              <EmptyTitle>Not loaded</EmptyTitle>
              <EmptyDescription>
                Nothing has been read for {label} yet.
              </EmptyDescription>
            </EmptyHeader>
            <EmptyContent>
              <Button variant="outline" onClick={onRefresh}>
                Load {label}
              </Button>
            </EmptyContent>
          </Empty>
        )}
      </div>
    </div>
  )
}
