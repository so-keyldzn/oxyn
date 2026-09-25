import * as React from "react"
import { HugeiconsIcon } from "@hugeicons/react"
import {
  Alert02Icon,
  CancelCircleIcon,
  DatabaseIcon,
  FlowchartIcon,
  MoreHorizontalIcon,
  PencilEdit02Icon,
  PlayIcon,
  RepeatIcon,
  TableIcon,
} from "@hugeicons/core-free-icons"

import { ColumnsMenu, ColumnsSubmenu } from "@/components/oxyn/columns-menu"
import { DensityMenu } from "@/components/oxyn/density-menu"
import { ResultFooter } from "@/components/oxyn/result-footer"
import { ResultGrid } from "@/components/oxyn/result-grid"
import type { FetchPage } from "@/components/oxyn/result-grid"
import type { GridPosition } from "@/components/oxyn/grid-selection"
import { rowCount } from "@/components/oxyn/status-bar"
import { Alert, AlertDescription, AlertTitle } from "@/components/ui/alert"
import { Badge } from "@/components/ui/badge"
import { Button } from "@/components/ui/button"
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuTrigger,
} from "@/components/ui/dropdown-menu"
import {
  Empty,
  EmptyContent,
  EmptyDescription,
  EmptyHeader,
  EmptyMedia,
  EmptyTitle,
} from "@/components/ui/empty"
import { Kbd, KbdGroup } from "@/components/ui/kbd"
import { Spinner } from "@/components/ui/spinner"
import type { Cell, ResultColumn } from "@/lib/ipc/types"
import type { DensityChoice } from "@/lib/ipc/settings"

/**
 * The five states of a view that depends on a remote operation
 * (docs/UX-SPEC.md, « États d'une vue »). The empty one is distinct from the
 * error, and « running » always carries a way to cancel.
 */
export type ResultState =
  | { status: "initial" }
  | {
      status: "running"
      rows: number
      /** Without server-side cancel, the button stays but promises less. */
      serverCancel: boolean
      /**
       * Known from the schema onward: with both, the grid is drawn while the
       * stream runs, and grows with `rows`.
       */
      result?: string | null
      columns?: Array<ResultColumn> | null
    }
  | {
      status: "populated"
      result: string
      columns: Array<ResultColumn>
      rows: number
      complete: boolean
      truncated: boolean
      cancelled: boolean
      elapsedMs?: number
    }
  | { status: "empty"; message: string }
  | { status: "error"; message: string; retryable: boolean }

/** Where a failed statement ran, for the line under the error title. */
export interface ErrorContext {
  connectionName: string
  statement?: string | null
}

const STATEMENT_PREVIEW = 80
const NO_HIDDEN_COLUMNS: ReadonlySet<number> = new Set()

/** The first line of a statement, on one line and bounded. Pure, so it is tested. */
export function statementPreview(statement: string) {
  const lines = statement
    .split(/\r?\n/)
    .map((line) => line.trim())
    .filter((line) => line !== "")
  const first = (lines[0] ?? "").replace(/\s+/g, " ")
  const cut = first.length > STATEMENT_PREVIEW
  const text = cut ? first.slice(0, STATEMENT_PREVIEW) : first
  return cut || lines.length > 1 ? `${text}…` : text
}

interface ResultPanelProps {
  state: ResultState
  fetchPage: FetchPage
  onCancel?: () => void
  onRetry?: () => void
  initialHint?: string
  /** Drawn above the rows of a populated result — the find bar, typically. */
  toolbar?: React.ReactNode
  /** Drawn at the right of the footer — the export menu, typically. */
  footerActions?: React.ReactNode
  /**
   * Below 1200 px: the footer draws one `Actions` menu holding `Columns` and
   * these items, and no `Columns` button of its own (docs/UX-SPEC.md,
   * « Largeur réduite »). `footerActions` is still drawn: at this width it
   * carries only what cannot wait in a menu, such as an export's progress.
   */
  compactActions?: React.ReactNode
  /** The text size, saved with the preferences; absent, no control is drawn. */
  density?: {
    value: DensityChoice
    onChange: (density: DensityChoice) => void
  }
  /** A quieter statement in the footer: « Total count not requested ». */
  footerNote?: React.ReactNode
  /** The connection and statement an error is about. */
  context?: ErrorContext
  /** Back to the statement, to fix it (⌘J in a console). */
  onEditQuery?: () => void
  matches?: ReadonlySet<number>
  reveal?: { row: number; key: number } | null
  onActiveChange?: (active: GridPosition | null, cell: Cell | undefined) => void
  onInspect?: (position: GridPosition) => void
  /**
   * The rows are the plan `Explain` asked for, not the statement's data: the
   * mention stays above them for as long as they are shown.
   */
  plan?: boolean
}

/**
 * Said in the result area itself: someone who ran `Explain`, looked away and
 * came back reads the grid, not the button they pressed (docs/UX-SPEC.md,
 * « Explain »).
 */
function PlanNote() {
  return (
    <p
      data-slot="result-plan-note"
      className="flex shrink-0 items-center gap-2 border-b px-3 py-1.5 text-xs text-muted-foreground"
    >
      <Badge variant="outline" className="shrink-0">
        <HugeiconsIcon
          icon={FlowchartIcon}
          strokeWidth={2}
          data-icon="inline-start"
        />
        Execution plan
      </Badge>
      <span className="min-w-0">
        How the server would run the statement, not its rows. The statement
        itself was not run.
      </span>
    </p>
  )
}

export const ResultPanel = React.memo(function ResultPanel({
  state,
  fetchPage,
  onCancel,
  onRetry,
  initialHint = "Run a statement to see its rows here.",
  toolbar,
  footerActions,
  compactActions,
  density,
  footerNote,
  context,
  onEditQuery,
  matches,
  reveal,
  onActiveChange,
  onInspect,
  plan = false,
}: ResultPanelProps) {
  const planNote = plan ? <PlanNote /> : null
  // Hidden columns belong to one result: a new statement brings other
  // columns under the same Arrow indexes.
  const resultKey =
    state.status === "populated" || state.status === "running"
      ? (state.result ?? null)
      : null
  const [hidden, setHidden] = React.useState<{
    result: string | null
    columns: ReadonlySet<number>
  }>({ result: resultKey, columns: new Set() })
  const hiddenColumns =
    hidden.result === resultKey ? hidden.columns : NO_HIDDEN_COLUMNS
  const columnsChoice = (columns: Array<ResultColumn>) => ({
    columns,
    hidden: hiddenColumns,
    onHiddenChange: (next: ReadonlySet<number>) =>
      setHidden({ result: resultKey, columns: next }),
  })
  // Offered where rows are drawn: it changes their height and text.
  const densityMenu = density ? <DensityMenu {...density} /> : null
  // What the footer offers besides its own statement: `columns` is null where
  // no grid is drawn, so that hiding a column or resizing text changes nothing.
  const barActions = (columns: Array<ResultColumn> | null) =>
    compactActions === undefined ? (
      <>
        {columns ? densityMenu : null}
        {columns ? <ColumnsMenu {...columnsChoice(columns)} /> : null}
        {footerActions}
      </>
    ) : (
      <>
        {columns ? densityMenu : null}
        {footerActions}
        <DropdownMenu>
          <DropdownMenuTrigger render={<Button variant="outline" size="xs" />}>
            <HugeiconsIcon
              icon={MoreHorizontalIcon}
              strokeWidth={2}
              data-icon="inline-start"
            />
            Actions
          </DropdownMenuTrigger>
          <DropdownMenuContent align="end" className="w-56">
            {columns ? <ColumnsSubmenu {...columnsChoice(columns)} /> : null}
            {compactActions}
          </DropdownMenuContent>
        </DropdownMenu>
      </>
    )

  switch (state.status) {
    case "initial":
      return (
        <Empty className="h-full border-0">
          <EmptyHeader>
            <EmptyMedia variant="icon">
              <HugeiconsIcon icon={PlayIcon} strokeWidth={2} />
            </EmptyMedia>
            <EmptyTitle>No result yet</EmptyTitle>
            <EmptyDescription>
              {initialHint}{" "}
              <KbdGroup>
                <Kbd>⌘</Kbd>
                <Kbd>↵</Kbd>
              </KbdGroup>
            </EmptyDescription>
          </EmptyHeader>
        </Empty>
      )

    case "running":
      if (state.result && state.columns) {
        return (
          <div className="flex h-full min-h-0 flex-col">
            {planNote}
            {toolbar}
            <ResultGrid
              resultKey={state.result}
              columns={state.columns}
              rowCount={state.rows}
              fetchPage={fetchPage}
              className="flex-1"
              hiddenColumns={hiddenColumns}
              matches={matches}
              reveal={reveal}
              onActiveChange={onActiveChange}
              onInspect={onInspect}
            />
            <ResultFooter
              state={{
                status: "running",
                rows: state.rows,
                serverCancel: state.serverCancel,
              }}
              note={
                state.rows === 0 ? "Waiting for the server's first rows." : null
              }
              actions={
                // At wide width a running result offers no export yet.
                compactActions === undefined ? (
                  <>
                    {densityMenu}
                    <ColumnsMenu {...columnsChoice(state.columns)} />
                  </>
                ) : (
                  barActions(state.columns)
                )
              }
              onCancel={onCancel}
            />
          </div>
        )
      }
      return (
        <Empty className="h-full border-0">
          <EmptyHeader>
            <EmptyMedia variant="icon">
              <Spinner />
            </EmptyMedia>
            <EmptyTitle>Running…</EmptyTitle>
            <EmptyDescription>
              {state.rows > 0
                ? `${rowCount(state.rows)} received so far.`
                : "Waiting for the server's first rows."}
            </EmptyDescription>
          </EmptyHeader>
          {onCancel ? (
            <EmptyContent>
              <Button variant="outline" onClick={onCancel}>
                <HugeiconsIcon
                  icon={CancelCircleIcon}
                  strokeWidth={2}
                  data-icon="inline-start"
                />
                Cancel
              </Button>
              {!state.serverCancel ? (
                <p className="max-w-sm text-center text-xs text-muted-foreground">
                  This session cannot send a cancel request to a server. Cancel
                  stops reading rows here.
                </p>
              ) : null}
            </EmptyContent>
          ) : null}
        </Empty>
      )

    case "populated": {
      const footer = (actions: React.ReactNode) => (
        <ResultFooter
          state={{
            status: "done",
            rows: state.rows,
            elapsedMs: state.elapsedMs,
            truncated: state.truncated,
            cancelled: state.cancelled,
          }}
          note={footerNote}
          actions={actions}
        />
      )
      if (state.rows === 0) {
        return (
          <div className="flex h-full min-h-0 flex-col">
            {planNote}
            <Empty className="flex-1 border-0">
              <EmptyHeader>
                <EmptyMedia variant="icon">
                  <HugeiconsIcon icon={TableIcon} strokeWidth={2} />
                </EmptyMedia>
                <EmptyTitle>No rows</EmptyTitle>
                <EmptyDescription>
                  {state.cancelled
                    ? "The statement was cancelled before any row arrived."
                    : state.columns.length > 0
                      ? `The statement returned ${state.columns.length} ${state.columns.length === 1 ? "column" : "columns"} and no row.`
                      : "The statement completed without returning rows."}
                </EmptyDescription>
              </EmptyHeader>
            </Empty>
            {footer(barActions(null))}
          </div>
        )
      }
      return (
        // The registry's `grid` zone: ⌘F there reaches the find field of
        // the toolbar (docs/adr/0041-registre-d-actions-menus-et-raccourcis.md).
        <div data-action-zone="grid" className="flex h-full min-h-0 flex-col">
          {planNote}
          {toolbar}
          <ResultGrid
            resultKey={state.result}
            columns={state.columns}
            rowCount={state.rows}
            fetchPage={fetchPage}
            className="flex-1"
            hiddenColumns={hiddenColumns}
            matches={matches}
            reveal={reveal}
            onActiveChange={onActiveChange}
            onInspect={onInspect}
          />
          {footer(barActions(state.columns))}
        </div>
      )
    }

    case "empty":
      return (
        <Empty className="h-full border-0">
          <EmptyHeader>
            <EmptyMedia variant="icon">
              <HugeiconsIcon icon={DatabaseIcon} strokeWidth={2} />
            </EmptyMedia>
            <EmptyTitle>Done</EmptyTitle>
            <EmptyDescription>{state.message}</EmptyDescription>
          </EmptyHeader>
        </Empty>
      )

    case "error":
      return (
        <div className="flex h-full min-h-0 flex-col overflow-auto p-4">
          <Alert className="gap-1.5 px-3 py-3">
            <HugeiconsIcon
              icon={Alert02Icon}
              strokeWidth={2}
              className="text-destructive"
            />
            <AlertTitle className="text-destructive">
              The statement failed
            </AlertTitle>
            <AlertDescription className="flex min-w-0 flex-col gap-2 text-balance">
              {context ? (
                <p className="min-w-0 truncate text-xs">
                  on{" "}
                  <span className="font-medium text-foreground">
                    {context.connectionName}
                  </span>
                  {context.statement ? (
                    <>
                      {" · "}
                      <code className="font-mono">
                        {statementPreview(context.statement)}
                      </code>
                    </>
                  ) : null}
                </p>
              ) : null}
              {/* The server's words, code included — never a paraphrase. */}
              <pre
                data-selectable
                tabIndex={0}
                aria-label="Server message"
                // `wrap-anywhere`: a long identifier must not widen the
                // Alert's grid column past the panel.
                className="max-h-64 overflow-auto rounded-md border bg-muted/40 p-2.5 font-mono text-xs wrap-anywhere whitespace-pre-wrap text-foreground outline-none focus-visible:ring-2 focus-visible:ring-ring"
              >
                {state.message}
              </pre>
              <p className="text-xs">
                {state.retryable
                  ? "This error is transient: running it again may succeed."
                  : "Running it again as is will fail the same way."}
              </p>
              {(state.retryable && onRetry) || onEditQuery ? (
                <div className="flex flex-wrap items-center gap-2">
                  {state.retryable && onRetry ? (
                    <Button size="sm" variant="outline" onClick={onRetry}>
                      <HugeiconsIcon
                        icon={RepeatIcon}
                        strokeWidth={2}
                        data-icon="inline-start"
                      />
                      Run again
                    </Button>
                  ) : null}
                  {onEditQuery ? (
                    <Button size="sm" variant="outline" onClick={onEditQuery}>
                      <HugeiconsIcon
                        icon={PencilEdit02Icon}
                        strokeWidth={2}
                        data-icon="inline-start"
                      />
                      Edit query
                      <KbdGroup className="ml-1">
                        <Kbd>⌘</Kbd>
                        <Kbd>J</Kbd>
                      </KbdGroup>
                    </Button>
                  ) : null}
                </div>
              ) : null}
            </AlertDescription>
          </Alert>
        </div>
      )
  }
})
