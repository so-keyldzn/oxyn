import * as React from "react"
import { HugeiconsIcon } from "@hugeicons/react"
import {
  Alert02Icon,
  ArrowExpand01Icon,
  ChartBarLineIcon,
  TimeQuarterPassIcon,
} from "@hugeicons/core-free-icons"

import { AssistantResultChart } from "@/components/oxyn/assistant-result-chart"
import { chartPlan } from "@/components/oxyn/result-chart-model"
import { HEADER_HEIGHT, ResultGrid } from "@/components/oxyn/result-grid"
import type { FetchPage } from "@/components/oxyn/result-grid"
import { Alert, AlertDescription, AlertTitle } from "@/components/ui/alert"
import { Button } from "@/components/ui/button"
import { Spinner } from "@/components/ui/spinner"
import type { ResultColumn } from "@/lib/ipc/types"

/**
 * Rows the chat grid reaches at most. The rest is one click away, in a result
 * tab: a conversation is read, not scrolled through a million rows.
 */
export const CHAT_ROWS = 100

/** Rows drawn before the chat grid scrolls. */
const VISIBLE_ROWS = 8

/** Room for a horizontal scrollbar under the last row. */
const SCROLLBAR_ROOM = 14

export type ToolRowsState =
  | { status: "loading" }
  | {
      status: "open"
      result: string
      columns: Array<ResultColumn>
      rows: number
      /** The result stopped at the row limit: these are not all its rows. */
      truncated: boolean
    }
  /** Retention released the buffer: nothing is rerun to bring it back. */
  | { status: "expired" }
  | { status: "error"; message: string; retryable: boolean }

/** The interface speaks English: its numbers too, as in the approval dialog. */
const count = new Intl.NumberFormat("en-US")

function counted(rows: number) {
  return `${count.format(rows)} ${rows === 1 ? "row" : "rows"}`
}

/** What the footer says of the rows shown. Pure, so it is tested. */
export function rowsCaption(rows: number, truncated: boolean) {
  const shown = Math.min(rows, CHAT_ROWS)
  const head =
    shown < rows
      ? `First ${counted(shown)} of ${count.format(rows)}`
      : counted(rows)
  return truncated ? `${head}, cut at the row limit` : head
}

/**
 * The rows an agent's query returned, under its tool call — for the user only.
 *
 * The model was told the shape of the result, never these values (ADR-0006,
 * ADR-0030 §4). The grid is the console's own: pages are read from the
 * backend's buffer as they scroll into view, and cells arrive formatted by
 * Rust. Nothing here reruns the query — an expired result says so.
 */
export function AssistantToolRows({
  state,
  fetchPage,
  onOpenAll,
  onRetry,
}: {
  state: ToolRowsState
  fetchPage: FetchPage
  /** Opens the whole result in a workspace tab. Runs nothing. */
  onOpenAll?: () => void
  /** Only for an error the backend declared retryable. */
  onRetry?: () => void
}) {
  const [charted, setCharted] = React.useState(false)
  const columns = state.status === "open" ? state.columns : null
  const chartRows =
    state.status === "open" ? Math.min(state.rows, CHAT_ROWS) : 0
  // Offered only when the columns allow one: a button that draws nothing
  // would be a control that lies about what it offers.
  const plan = React.useMemo(
    () => (columns ? chartPlan(columns, chartRows) : null),
    [columns, chartRows]
  )
  switch (state.status) {
    case "loading":
      return (
        <p
          data-slot="assistant-tool-rows"
          aria-busy
          className="flex items-center gap-2 border-t px-3 py-2 text-xs text-muted-foreground"
        >
          <Spinner aria-hidden />
          Reading the rows Oxyn kept…
        </p>
      )
    case "expired":
      return (
        <p
          data-slot="assistant-tool-rows"
          className="flex items-start gap-1.5 border-t px-3 py-2 text-xs text-muted-foreground"
        >
          <HugeiconsIcon
            icon={TimeQuarterPassIcon}
            strokeWidth={2}
            className="mt-0.5 size-3.5 shrink-0"
            aria-hidden
          />
          Result no longer available: Oxyn released these rows. Nothing is rerun
          to bring them back.
        </p>
      )
    case "error":
      return (
        <div data-slot="assistant-tool-rows" className="border-t p-2">
          <Alert variant="destructive">
            <HugeiconsIcon icon={Alert02Icon} strokeWidth={2} />
            <AlertTitle>
              The rows could not be read. Nothing was rerun.
            </AlertTitle>
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
    case "open": {
      if (state.rows === 0)
        return (
          <p
            data-slot="assistant-tool-rows"
            className="border-t px-3 py-2 text-xs text-muted-foreground"
          >
            The query returned no rows.
          </p>
        )
      const shown = Math.min(state.rows, CHAT_ROWS)
      const visible = Math.min(shown, VISIBLE_ROWS)
      return (
        <div
          data-slot="assistant-tool-rows"
          className="flex min-w-0 flex-col border-t"
        >
          {charted && plan ? (
            <AssistantResultChart
              plan={plan}
              columns={state.columns}
              rows={shown}
              fetchPage={fetchPage}
            />
          ) : (
            <div
              className="flex min-h-0 flex-col"
              style={{
                height: `calc(${HEADER_HEIGHT + SCROLLBAR_ROOM}px + ${visible} * var(--grid-row-height, 24px))`,
              }}
            >
              <ResultGrid
                resultKey={state.result}
                columns={state.columns}
                rowCount={shown}
                fetchPage={fetchPage}
                className="flex-1"
                aria-label="Rows the query returned"
              />
            </div>
          )}
          <div className="flex flex-wrap items-center gap-x-2 gap-y-1 border-t px-3 py-1.5 text-xs text-muted-foreground">
            <span className="tabular-nums">
              {rowsCaption(state.rows, state.truncated)}
            </span>
            <span>· shown to you only; the model got the count</span>
            {plan ? (
              <Button
                size="xs"
                variant={charted ? "secondary" : "ghost"}
                aria-pressed={charted}
                className="ml-auto"
                onClick={() => setCharted((on) => !on)}
              >
                <HugeiconsIcon
                  icon={ChartBarLineIcon}
                  strokeWidth={2}
                  data-icon="inline-start"
                />
                Chart
              </Button>
            ) : null}
            {onOpenAll ? (
              <Button
                size="xs"
                variant="outline"
                className={plan ? undefined : "ml-auto"}
                onClick={onOpenAll}
              >
                <HugeiconsIcon
                  icon={ArrowExpand01Icon}
                  strokeWidth={2}
                  data-icon="inline-start"
                />
                Open all rows
              </Button>
            ) : null}
          </div>
        </div>
      )
    }
  }
}
