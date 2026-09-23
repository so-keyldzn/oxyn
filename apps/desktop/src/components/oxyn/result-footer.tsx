import type * as React from "react"
import { HugeiconsIcon } from "@hugeicons/react"
import { Alert02Icon, CancelCircleIcon } from "@hugeicons/core-free-icons"

import { Button } from "@/components/ui/button"
import { Spinner } from "@/components/ui/spinner"
import { cn } from "@/lib/utils"

export type ResultFooterState =
  | {
      status: "running"
      rows: number
      /** Without server-side cancel, Cancel stops reading here only. */
      serverCancel: boolean
    }
  | {
      status: "done"
      rows: number
      elapsedMs?: number | null
      truncated: boolean
      cancelled: boolean
    }

/** « 840 ms », « 12.4 s », « 3 min 05 s ». Pure, so it is tested. */
export function formatElapsed(ms: number) {
  if (ms < 1000) return `${Math.round(ms)} ms`
  if (ms < 60_000) return `${(ms / 1000).toFixed(1)} s`
  const minutes = Math.floor(ms / 60_000)
  const seconds = Math.floor((ms % 60_000) / 1000)
  return `${minutes} min ${String(seconds).padStart(2, "0")} s`
}

function rows(count: number) {
  return `${count.toLocaleString("en-US")} ${count === 1 ? "row" : "rows"}`
}

/** The sentence the footer says, and whether it is a warning. Pure. */
export function footerSummary(state: ResultFooterState): {
  text: string
  warning: boolean
} {
  if (state.status === "running") {
    return { text: `${rows(state.rows)} received · running`, warning: false }
  }
  const elapsed =
    state.elapsedMs == null ? "" : ` · ${formatElapsed(state.elapsedMs)}`
  if (state.cancelled) {
    return {
      text: `Cancelled · ${rows(state.rows)} shown, not the whole result${elapsed}`,
      warning: true,
    }
  }
  if (state.truncated) {
    return {
      text: `Truncated · ${rows(state.rows)} shown, not the whole result${elapsed}`,
      warning: true,
    }
  }
  return { text: `${rows(state.rows)} shown${elapsed}`, warning: false }
}

/**
 * The bar under a result: what is shown, how long it took, whether it is the
 * whole result, and what can be done with it (docs/UX-SPEC.md, « Lisibilité
 * et hauteur de grille » — status and export scope sit under their area).
 *
 * Only the settled summary is a live region: a row counter announced at every
 * batch would drown a screen reader.
 */
export function ResultFooter({
  state,
  note,
  actions,
  onCancel,
  className,
}: {
  state: ResultFooterState
  /** A second, quieter statement: « Total count not requested », a page. */
  note?: React.ReactNode
  /** Export and the like, at the right. */
  actions?: React.ReactNode
  onCancel?: () => void
  className?: string
}) {
  const summary = footerSummary(state)
  const running = state.status === "running"
  return (
    <div
      data-slot="result-footer"
      className={cn(
        "flex min-h-8 shrink-0 flex-wrap items-center gap-x-3 gap-y-1 border-t bg-card px-2 py-1 text-xs",
        className
      )}
    >
      <span className="flex min-w-0 items-center gap-1.5">
        {running ? (
          // Decorative here: the « Running » region beside it speaks once.
          <Spinner className="size-3" role="presentation" aria-hidden />
        ) : summary.warning ? (
          <HugeiconsIcon
            icon={Alert02Icon}
            strokeWidth={2}
            className="size-3.5 shrink-0 text-warning"
          />
        ) : null}
        {running ? (
          <>
            <span className="truncate text-foreground tabular-nums" aria-hidden>
              {summary.text}
            </span>
            <span role="status" className="sr-only">
              Running
            </span>
          </>
        ) : (
          <span
            role="status"
            // « Truncated · … not the whole result » must stay readable when
            // the bar is too narrow for it.
            title={summary.text}
            className={cn(
              "truncate tabular-nums",
              summary.warning ? "text-warning" : "text-foreground"
            )}
          >
            {summary.text}
          </span>
        )}
      </span>
      {note ? (
        <span className="min-w-0 truncate text-muted-foreground">{note}</span>
      ) : null}
      <span className="ml-auto flex min-w-0 flex-wrap items-center justify-end gap-2">
        {running && !state.serverCancel ? (
          // The reserve goes with the button at every width: hidden on a
          // narrow window, Cancel would promise what this session cannot do
          // (docs/UX-SPEC.md, « Annulation »). It wraps, it never disappears.
          <span className="min-w-0 text-muted-foreground">
            Cancel stops reading here; the server may keep running.
          </span>
        ) : null}
        {running && onCancel ? (
          <Button size="xs" variant="outline" onClick={onCancel}>
            <HugeiconsIcon
              icon={CancelCircleIcon}
              strokeWidth={2}
              data-icon="inline-start"
            />
            Cancel
          </Button>
        ) : null}
        {actions}
      </span>
    </div>
  )
}
