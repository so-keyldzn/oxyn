import * as React from "react"
import { HugeiconsIcon } from "@hugeicons/react"
import {
  Alert02Icon,
  ArrowExpand01Icon,
  ArrowLeft01Icon,
  ArrowRight01Icon,
  ChartBarLineIcon,
  TimeQuarterPassIcon,
} from "@hugeicons/core-free-icons"
import type { IconSvgElement } from "@hugeicons/react"

import { AssistantResultChart } from "@/components/oxyn/assistant-result-chart"
import {
  clampPage,
  pageCount,
  pageFetch,
  pageRange,
  rangeText,
  rowsCaption,
} from "@/components/oxyn/assistant-tool-rows-pages"
import { gridLayoutStore, schemaOf } from "@/components/oxyn/grid-layout"
import { chartPlan } from "@/components/oxyn/result-chart-model"
import { HEADER_HEIGHT, ResultGrid } from "@/components/oxyn/result-grid"
import type { FetchPage } from "@/components/oxyn/result-grid"
import { Alert, AlertDescription, AlertTitle } from "@/components/ui/alert"
import { Button } from "@/components/ui/button"
import { Spinner } from "@/components/ui/spinner"
import {
  Tooltip,
  TooltipContent,
  TooltipTrigger,
} from "@/components/ui/tooltip"
import { BackendError } from "@/lib/ipc/client"
import type { ResultColumn } from "@/lib/ipc/types"
import { cn } from "@/lib/utils"

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

/** Where the reader stands, for the result it was chosen on. */
interface Paging {
  result: string | null
  page: number
  /** Bumped by « Try again »: the page is read again, nothing else. */
  attempt: number
  /** Said once the user moves, never on every scroll of the grid. */
  announced: string
}

function firstPage(result: string | null): Paging {
  return { result, page: 0, attempt: 0, announced: "" }
}

/** A page read that failed, for the page it was read for. */
interface Failure {
  key: string
  message: string
  retryable: boolean
}

function Released({ id }: { id?: string }) {
  return (
    <p
      id={id}
      className="flex items-start gap-1.5 px-3 py-2 text-xs text-muted-foreground"
    >
      <HugeiconsIcon
        icon={TimeQuarterPassIcon}
        strokeWidth={2}
        className="mt-0.5 size-3.5 shrink-0"
        aria-hidden
      />
      Result no longer available: Oxyn released these rows. Nothing is rerun to
      bring them back.
    </p>
  )
}

/** The backend's words, whole; « Try again » only because it said so. */
function ReadFailure({
  message,
  retryable,
  onRetry,
}: {
  message: string
  retryable: boolean
  onRetry?: () => void
}) {
  return (
    <div className="p-2">
      <Alert variant="destructive">
        <HugeiconsIcon icon={Alert02Icon} strokeWidth={2} />
        <AlertTitle>The rows could not be read. Nothing was rerun.</AlertTitle>
        <AlertDescription>
          <p dir="auto" className="font-mono break-words">
            {message}
          </p>
          {retryable && onRetry ? (
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
}

/**
 * A pager button: inert at a bound, not hidden, and still focusable — the
 * keyboard keeps its place when the last page disables « Next page ».
 */
function PageButton({
  label,
  icon,
  blocked,
  onClick,
}: {
  label: string
  icon: IconSvgElement
  /** Why the button does nothing, when it does nothing. */
  blocked: string | null
  onClick: () => void
}) {
  const reasonId = React.useId()
  return (
    <Tooltip>
      <TooltipTrigger
        render={
          <Button
            size="icon-xs"
            variant="ghost"
            aria-label={label}
            aria-describedby={blocked ? reasonId : undefined}
            disabled={blocked !== null}
            focusableWhenDisabled
            className="data-disabled:opacity-50"
            onClick={onClick}
          />
        }
      >
        <HugeiconsIcon icon={icon} strokeWidth={2} />
      </TooltipTrigger>
      <TooltipContent>{blocked ?? label}</TooltipContent>
      {blocked ? (
        <span id={reasonId} className="sr-only">
          {blocked}
        </span>
      ) : null}
    </Tooltip>
  )
}

/**
 * A footer action once the rows it works on were released: still drawn and
 * focusable, so nothing moves under the keyboard, inert, and described by the
 * message that says why. Its tooltip only exists then — the label says the
 * rest — but it is always mounted, so focus is not lost when the rows go.
 */
function FooterAction({
  releasedBy,
  onClick,
  children,
  className,
  ...props
}: Omit<React.ComponentProps<typeof Button>, "disabled"> & {
  /** The id of the message saying the rows were released, once they were. */
  releasedBy: string | null
}) {
  return (
    <Tooltip disabled={releasedBy === null}>
      <TooltipTrigger
        render={
          <Button
            {...props}
            aria-describedby={releasedBy ?? undefined}
            disabled={releasedBy !== null}
            focusableWhenDisabled
            className={cn("data-disabled:opacity-50", className)}
            onClick={releasedBy === null ? onClick : undefined}
          />
        }
      >
        {children}
      </TooltipTrigger>
      <TooltipContent>Result no longer available</TooltipContent>
    </Tooltip>
  )
}

/**
 * The rows an agent's query returned, under its tool call — for the user only.
 *
 * The model was told the shape of the result, never these values (ADR-0006,
 * ADR-0030 §4). The grid is the console's own, over one page of `CHAT_ROWS`
 * rows at a time: Previous and Next read another page from the backend's
 * buffer, in the order it holds them, and cells arrive formatted by Rust.
 * Nothing here reruns the query or opens the result again — an expired result
 * says so.
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
  const open = state.status === "open" ? state : null
  const result = open?.result ?? null
  const rows = open?.rows ?? 0
  const columns = open?.columns ?? null

  // Another result — another call, a retry — starts on its first page.
  const [paging, setPaging] = React.useState(() => firstPage(result))
  if (paging.result !== result) setPaging(firstPage(result))
  const current = paging.result === result ? paging : firstPage(result)
  const page = clampPage(current.page, rows)
  const range = pageRange(page, rows)
  const pages = pageCount(rows)
  // The page's identity carries the columns: other columns under the same
  // result are another grid, whose widths and order start over.
  const schema = columns ? schemaOf(columns) : ""
  const pageKey = `${result ?? ""}:${schema}:${range.start}:${current.attempt}`

  // Widths and order belong to the result, not to the page: the grid is
  // remounted on each page and finds them here. Gone with this view.
  const [layoutStore] = React.useState(gridLayoutStore)
  const layout = React.useMemo(
    () => (result === null ? undefined : { store: layoutStore, key: result }),
    [layoutStore, result]
  )
  const releasedId = React.useId()

  // Expiry is the result's, whatever page found it; a failure is the page's,
  // and a read for a page already left never shows over the one shown.
  const [expiredOn, setExpiredOn] = React.useState<string | null>(null)
  const [failure, setFailure] = React.useState<Failure | null>(null)
  const [reading, setReading] = React.useState({ key: "", count: 0 })
  const expired = result !== null && expiredOn === result
  const failed = failure?.key === pageKey ? failure : null
  const busy = reading.key === pageKey && reading.count > 0

  const { start, end } = range
  const shownFetch = React.useMemo<FetchPage>(() => {
    const read = pageFetch(fetchPage, { start, end })
    const settle = () =>
      setReading((now) =>
        now.key === pageKey
          ? { key: pageKey, count: Math.max(0, now.count - 1) }
          : now
      )
    return async (offset, limit) => {
      setReading((now) => ({
        key: pageKey,
        count: now.key === pageKey ? now.count + 1 : 1,
      }))
      try {
        const answer = await read(offset, limit)
        if (answer.type === "expired") setExpiredOn(result)
        return answer
      } catch (error) {
        setFailure({
          key: pageKey,
          message: error instanceof Error ? error.message : String(error),
          retryable: error instanceof BackendError && error.retryable,
        })
        throw error
      } finally {
        settle()
      }
    }
  }, [fetchPage, start, end, pageKey, result])

  const chartRows = end - start
  // Offered only when the columns allow one: a button that draws nothing
  // would be a control that lies about what it offers.
  const plan = React.useMemo(
    () => (columns ? chartPlan(columns, chartRows) : null),
    [columns, chartRows]
  )

  const go = (next: number) => {
    const target = clampPage(next, rows)
    if (target === page) return
    setPaging({
      result,
      page: target,
      attempt: 0,
      announced: `Showing rows ${rangeText(pageRange(target, rows), rows)}`,
    })
  }

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
        <div data-slot="assistant-tool-rows" className="border-t">
          <Released />
        </div>
      )
    case "error":
      return (
        <div data-slot="assistant-tool-rows" className="border-t">
          <ReadFailure
            message={state.message}
            retryable={state.retryable}
            onRetry={onRetry}
          />
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
      const paged = pages > 1
      const shown = rangeText(range, rows)
      const visible = Math.min(end - start, VISIBLE_ROWS)
      const stopped = expired ? "Result no longer available" : null
      return (
        <div
          data-slot="assistant-tool-rows"
          className="flex min-w-0 flex-col border-t"
        >
          {/* Mounted empty, so the release is announced once, politely,
              wherever focus is; it takes the place of the grid as of the
              chart — neither stays drawn from rows Oxyn no longer holds. */}
          <div role="status">
            {expired ? <Released id={releasedId} /> : null}
          </div>
          {expired ? null : failed ? (
            <ReadFailure
              message={failed.message}
              retryable={failed.retryable}
              onRetry={() => {
                setFailure(null)
                setPaging({ ...current, attempt: current.attempt + 1 })
              }}
            />
          ) : charted && plan ? (
            <>
              <AssistantResultChart
                key={pageKey}
                plan={plan}
                columns={state.columns}
                rows={chartRows}
                fetchPage={shownFetch}
              />
              {paged ? (
                <p className="px-3 pb-2 text-xs text-muted-foreground tabular-nums">
                  Drawn from the page shown: rows {shown}.
                </p>
              ) : null}
            </>
          ) : (
            <div
              className="flex min-h-0 flex-col"
              aria-busy={busy || undefined}
              style={{
                height: `calc(${HEADER_HEIGHT + SCROLLBAR_ROOM}px + ${visible} * var(--grid-row-height, 24px))`,
              }}
            >
              <ResultGrid
                // Selection, active cell and scroll start clean on each page,
                // and a page's reads are cached under it alone; its widths
                // and order are the result's.
                key={pageKey}
                resultKey={pageKey}
                layout={layout}
                columns={state.columns}
                rowCount={end - start}
                firstRow={start}
                fetchPage={shownFetch}
                className="flex-1"
                aria-label={
                  paged
                    ? `Rows the query returned, ${shown}`
                    : "Rows the query returned"
                }
              />
            </div>
          )}
          <div className="flex flex-col gap-1 border-t px-3 py-1.5 text-xs text-muted-foreground">
            {/* Where the rows are on the left, what can be done with them on
                the right: the actions wrap together, never one by one. */}
            <div className="flex flex-wrap items-center gap-x-2 gap-y-1">
              <span className="tabular-nums">
                {rowsCaption(range, rows, state.truncated)}
              </span>
              {paged ? (
                <span className="flex items-center gap-0.5">
                  <PageButton
                    label="Previous page"
                    icon={ArrowLeft01Icon}
                    blocked={
                      stopped ?? (page === 0 ? "This is the first page" : null)
                    }
                    onClick={() => go(page - 1)}
                  />
                  <PageButton
                    label="Next page"
                    icon={ArrowRight01Icon}
                    blocked={
                      stopped ??
                      (page >= pages - 1 ? "This is the last page" : null)
                    }
                    onClick={() => go(page + 1)}
                  />
                  <span className="sr-only" aria-live="polite" aria-atomic>
                    {current.announced}
                  </span>
                </span>
              ) : null}
              {plan || onOpenAll ? (
                <span className="ml-auto flex items-center gap-1">
                  {plan ? (
                    <FooterAction
                      releasedBy={expired ? releasedId : null}
                      size="xs"
                      variant={charted ? "secondary" : "ghost"}
                      aria-pressed={charted}
                      onClick={() => setCharted((on) => !on)}
                    >
                      <HugeiconsIcon
                        icon={ChartBarLineIcon}
                        strokeWidth={2}
                        data-icon="inline-start"
                      />
                      Chart
                    </FooterAction>
                  ) : null}
                  {onOpenAll ? (
                    <FooterAction
                      releasedBy={expired ? releasedId : null}
                      size="xs"
                      variant="outline"
                      onClick={onOpenAll}
                    >
                      <HugeiconsIcon
                        icon={ArrowExpand01Icon}
                        strokeWidth={2}
                        data-icon="inline-start"
                      />
                      Open all rows
                    </FooterAction>
                  ) : null}
                </span>
              ) : null}
            </div>
            {/* The text stays as the spec quotes it; the capital is drawn. */}
            <p className="first-letter:uppercase">
              shown to you only; the model got the count
            </p>
          </div>
        </div>
      )
    }
  }
}
