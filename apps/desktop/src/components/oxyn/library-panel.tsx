import * as React from "react"
import { HugeiconsIcon } from "@hugeicons/react"
import {
  Alert02Icon,
  ArrowLeft01Icon,
  ArrowRight01Icon,
  CheckmarkCircle02Icon,
  Clock01Icon,
  Copy01Icon,
  Delete02Icon,
  FileEditIcon,
  RefreshIcon,
  Search01Icon,
  TableIcon,
} from "@hugeicons/core-free-icons"

import {
  AlertDialog,
  AlertDialogCancel,
  AlertDialogContent,
  AlertDialogDescription,
  AlertDialogFooter,
  AlertDialogHeader,
  AlertDialogTitle,
} from "@/components/ui/alert-dialog"
import { Alert, AlertDescription, AlertTitle } from "@/components/ui/alert"
import { Badge } from "@/components/ui/badge"
import { Button } from "@/components/ui/button"
import {
  Empty,
  EmptyDescription,
  EmptyHeader,
  EmptyTitle,
} from "@/components/ui/empty"
import {
  InputGroup,
  InputGroupAddon,
  InputGroupInput,
} from "@/components/ui/input-group"
import { NativeSelect, NativeSelectOption } from "@/components/ui/native-select"
import { Skeleton } from "@/components/ui/skeleton"
import { ToggleGroup, ToggleGroupItem } from "@/components/ui/toggle-group"
import type {
  DocumentEntry,
  HistoryRow,
  HistoryStatusChoice,
} from "@/lib/ipc/library"
import { cn } from "@/lib/utils"

export type LibraryView = "history" | "saved"

export type LibraryState =
  | { status: "loading" }
  | { status: "error"; message: string }
  | { status: "history"; entries: Array<HistoryRow> }
  | { status: "saved"; entries: Array<DocumentEntry> }

export interface HistoryFilters {
  /** `""` is all connections. */
  connection: string
  days: number | null
  status: HistoryStatusChoice
}

export interface ConnectionOption {
  value: string
  label: string
}

const STATUSES: ReadonlyArray<{ value: HistoryStatusChoice; label: string }> = [
  { value: "all", label: "All statuses" },
  { value: "succeeded", label: "Succeeded" },
  { value: "failed", label: "Failed" },
  { value: "cancelled", label: "Cancelled" },
  { value: "denied", label: "Denied" },
  { value: "running", label: "Running" },
  { value: "ambiguous", label: "Ambiguous" },
]

const PERIODS = [
  { value: "7", label: "Last 7 days" },
  { value: "30", label: "Last 30 days" },
  { value: "all", label: "All dates" },
]

function when(iso: string) {
  // Shown in UTC, as recorded: the history is a log, not a calendar.
  return `${iso.slice(0, 16).replace("T", " ")} UTC`
}

/**
 * The local library: execution history and saved queries (ADR-0014).
 *
 * Opening an entry always copies its text into a new console; nothing runs.
 * An ambiguous write is marked for inspection and offers no copy: it is never
 * replayed (I-13). Once the user has inspected the server, they may mark it
 * reconciled, after a confirmation that names the connection and quotes the
 * statement. Deleting a saved query removes a local file, never a database
 * object, and asks first.
 */
/** A result is addressed through the session that produced it: this connection only. */
function retainsResult(row: HistoryRow, connection: string) {
  return row.result !== null && row.connection === connection
}

export function LibraryPanel({
  view,
  onViewChange,
  search,
  onSearchChange,
  filters,
  onFiltersChange,
  connections,
  state,
  hasPrevious,
  hasNext,
  onPrevious,
  onNext,
  onRefresh,
  currentConnection,
  onOpenHistory,
  onOpenResult,
  onReconcile,
  onOpenSaved,
  onResumeSaved,
  onDeleteSaved,
}: {
  view: LibraryView
  onViewChange: (view: LibraryView) => void
  search: string
  onSearchChange: (search: string) => void
  filters: HistoryFilters
  onFiltersChange: (filters: HistoryFilters) => void
  connections: Array<ConnectionOption>
  state: LibraryState
  hasPrevious: boolean
  hasNext: boolean
  onPrevious: () => void
  onNext: () => void
  onRefresh: () => void
  /** The connection of this workspace: only its queries can be resumed. */
  currentConnection: string
  onOpenHistory: (row: HistoryRow) => void
  /**
   * Opens the rows a run of this connection still retains. Reads them only:
   * nothing is rerun, even when they have expired.
   */
  onOpenResult?: (row: HistoryRow) => void
  /**
   * Declares a write inspected on the server, so launches stop warning about
   * it. Records a local fact only: nothing is sent and nothing is retried.
   */
  onReconcile: (row: HistoryRow) => void
  onOpenSaved: (entry: DocumentEntry) => void
  onResumeSaved: (entry: DocumentEntry) => void
  onDeleteSaved: (entry: DocumentEntry) => void
}) {
  const [deleting, setDeleting] = React.useState<DocumentEntry | null>(null)
  const [reconciling, setReconciling] = React.useState<HistoryRow | null>(null)
  const cancelRef = React.useRef<HTMLButtonElement>(null)
  const keepRef = React.useRef<HTMLButtonElement>(null)

  return (
    <div className="flex min-h-0 flex-1 flex-col gap-2 p-2">
      <ToggleGroup
        value={[view]}
        onValueChange={(value) => {
          const next = value[0]
          if (next === "history" || next === "saved") onViewChange(next)
        }}
        variant="outline"
        size="sm"
        className="w-full"
      >
        <ToggleGroupItem value="history" className="flex-1">
          History
        </ToggleGroupItem>
        <ToggleGroupItem value="saved" className="flex-1">
          Saved queries
        </ToggleGroupItem>
      </ToggleGroup>

      <InputGroup>
        <InputGroupAddon>
          <HugeiconsIcon icon={Search01Icon} strokeWidth={2} />
        </InputGroupAddon>
        <InputGroupInput
          aria-label="Search"
          placeholder="Search"
          value={search}
          onChange={(event) => onSearchChange(event.target.value)}
        />
      </InputGroup>

      {view === "history" ? (
        <div className="flex flex-col gap-1.5">
          <NativeSelect
            size="sm"
            aria-label="Connection"
            value={filters.connection}
            onChange={(event) =>
              onFiltersChange({ ...filters, connection: event.target.value })
            }
          >
            <NativeSelectOption value="">All connections</NativeSelectOption>
            {connections.map((option) => (
              <NativeSelectOption key={option.value} value={option.value}>
                {option.label}
              </NativeSelectOption>
            ))}
          </NativeSelect>
          <div className="flex gap-1.5">
            <NativeSelect
              size="sm"
              aria-label="Period"
              value={filters.days === null ? "all" : String(filters.days)}
              onChange={(event) =>
                onFiltersChange({
                  ...filters,
                  days:
                    event.target.value === "all"
                      ? null
                      : Number(event.target.value),
                })
              }
            >
              {PERIODS.map((period) => (
                <NativeSelectOption key={period.value} value={period.value}>
                  {period.label}
                </NativeSelectOption>
              ))}
            </NativeSelect>
            <NativeSelect
              size="sm"
              aria-label="Status"
              value={filters.status}
              onChange={(event) =>
                onFiltersChange({
                  ...filters,
                  status: event.target.value as HistoryStatusChoice,
                })
              }
            >
              {STATUSES.map((status) => (
                <NativeSelectOption key={status.value} value={status.value}>
                  {status.label}
                </NativeSelectOption>
              ))}
            </NativeSelect>
          </div>
          <p className="text-xs text-muted-foreground">
            Search query text · local history across workspaces
          </p>
        </div>
      ) : (
        <p className="text-xs text-muted-foreground">
          Search saved titles and queries · this workspace
        </p>
      )}

      <div className="min-h-0 flex-1 overflow-y-auto">
        {state.status === "loading" ? (
          <div className="flex flex-col gap-2" aria-busy="true">
            <Skeleton className="h-12" />
            <Skeleton className="h-12" />
            <Skeleton className="h-12" />
          </div>
        ) : state.status === "error" ? (
          <Alert variant="destructive">
            <AlertDescription>{state.message}</AlertDescription>
          </Alert>
        ) : state.entries.length === 0 ? (
          <Empty className="p-4">
            <EmptyHeader>
              <EmptyTitle>No queries match these filters.</EmptyTitle>
              <EmptyDescription>
                {state.status === "history"
                  ? "Queries you run appear here."
                  : "Save a console with ⌘S to find it here."}
              </EmptyDescription>
            </EmptyHeader>
          </Empty>
        ) : state.status === "history" ? (
          <ul className="flex flex-col gap-1">
            {state.entries.map((row) => (
              <li
                key={row.id}
                className="flex flex-col gap-1 rounded-md border p-2 text-xs"
              >
                <code className="line-clamp-2 font-mono break-all">
                  {row.preview}
                </code>
                <div className="flex items-center gap-1.5 text-muted-foreground">
                  <span dir="auto" className="min-w-0 truncate">
                    {row.connectionName ?? "Connection unavailable"}
                  </span>
                  <span>·</span>
                  <span className="shrink-0 tabular-nums">{when(row.at)}</span>
                </div>
                {/* Wraps: the panel is 280 px wide in the sidebar, and two
                    actions beside a badge do not fit on one line there. */}
                <div className="flex flex-wrap items-center gap-1.5">
                  {row.needsInspection ? (
                    <Badge variant="destructive">
                      <HugeiconsIcon
                        icon={Alert02Icon}
                        strokeWidth={2}
                        data-icon="inline-start"
                      />
                      Needs inspection
                    </Badge>
                  ) : (
                    <Badge variant="outline">{row.status}</Badge>
                  )}
                  {row.reconciled ? (
                    <Badge variant="secondary">Reconciled</Badge>
                  ) : null}
                  {row.durationMs !== null ? (
                    <span className="text-muted-foreground">
                      {row.durationMs.toLocaleString("en-US")} ms
                    </span>
                  ) : null}
                  {onOpenResult && retainsResult(row, currentConnection) ? (
                    <Button
                      size="xs"
                      variant="ghost"
                      className="ml-auto"
                      onClick={() => onOpenResult(row)}
                    >
                      <HugeiconsIcon
                        icon={TableIcon}
                        strokeWidth={2}
                        data-icon="inline-start"
                      />
                      Open result
                    </Button>
                  ) : null}
                  {row.needsInspection ? (
                    <Button
                      size="xs"
                      variant="ghost"
                      className={cn(
                        !(
                          onOpenResult && retainsResult(row, currentConnection)
                        ) && "ml-auto"
                      )}
                      onClick={() => setReconciling(row)}
                    >
                      <HugeiconsIcon
                        icon={CheckmarkCircle02Icon}
                        strokeWidth={2}
                        data-icon="inline-start"
                      />
                      Mark reconciled
                    </Button>
                  ) : (
                    <Button
                      size="xs"
                      variant="ghost"
                      className={cn(
                        !(
                          onOpenResult && retainsResult(row, currentConnection)
                        ) && "ml-auto"
                      )}
                      onClick={() => onOpenHistory(row)}
                    >
                      <HugeiconsIcon
                        icon={Copy01Icon}
                        strokeWidth={2}
                        data-icon="inline-start"
                      />
                      Open copy
                    </Button>
                  )}
                </div>
              </li>
            ))}
          </ul>
        ) : (
          <ul className="flex flex-col gap-1">
            {state.entries.map((entry) => (
              <li
                key={entry.id}
                className="flex flex-col gap-1 rounded-md border p-2 text-xs"
              >
                <span dir="auto" className="truncate font-medium">
                  {entry.fromAgent ? "AI · " : ""}
                  {entry.title === "" ? "Untitled query" : entry.title}
                </span>
                <div className="flex items-center gap-1.5 text-muted-foreground">
                  <span dir="auto" className="min-w-0 truncate">
                    {entry.connectionName ?? "No connection"}
                  </span>
                  <span>·</span>
                  <span>{entry.hasChanges ? "Draft changed" : "Saved"}</span>
                </div>
                <div className="flex flex-wrap items-center gap-1">
                  <Button
                    size="xs"
                    variant="ghost"
                    onClick={() => onOpenSaved(entry)}
                  >
                    <HugeiconsIcon
                      icon={Copy01Icon}
                      strokeWidth={2}
                      data-icon="inline-start"
                    />
                    Open copy
                  </Button>
                  {entry.connection === currentConnection ? (
                    <Button
                      size="xs"
                      variant="ghost"
                      onClick={() => onResumeSaved(entry)}
                    >
                      <HugeiconsIcon
                        icon={FileEditIcon}
                        strokeWidth={2}
                        data-icon="inline-start"
                      />
                      Resume
                    </Button>
                  ) : null}
                  <Button
                    size="icon-xs"
                    variant="ghost"
                    className="ml-auto"
                    aria-label={`Delete ${entry.title === "" ? "Untitled query" : entry.title}`}
                    onClick={() => setDeleting(entry)}
                  >
                    <HugeiconsIcon icon={Delete02Icon} strokeWidth={2} />
                  </Button>
                </div>
              </li>
            ))}
          </ul>
        )}
      </div>

      <div className="flex items-center gap-1">
        <Button
          size="icon-sm"
          variant="ghost"
          aria-label="Previous page"
          disabled={!hasPrevious || state.status === "loading"}
          onClick={onPrevious}
        >
          <HugeiconsIcon icon={ArrowLeft01Icon} strokeWidth={2} />
        </Button>
        <Button
          size="icon-sm"
          variant="ghost"
          aria-label="Next page"
          disabled={!hasNext || state.status === "loading"}
          onClick={onNext}
        >
          <HugeiconsIcon icon={ArrowRight01Icon} strokeWidth={2} />
        </Button>
        <Button
          size="sm"
          variant="ghost"
          className="ml-auto"
          onClick={onRefresh}
        >
          <HugeiconsIcon
            icon={RefreshIcon}
            strokeWidth={2}
            data-icon="inline-start"
          />
          Refresh
        </Button>
      </div>

      {view === "history" ? (
        <Alert>
          <HugeiconsIcon icon={Clock01Icon} strokeWidth={2} />
          <AlertTitle>Ambiguous writes are never replayed</AlertTitle>
          <AlertDescription>
            An expired write may have reached the server. History offers
            inspection and reconciliation, never a retry action.
          </AlertDescription>
        </Alert>
      ) : null}

      <AlertDialog
        open={deleting !== null}
        onOpenChange={(open) => {
          if (!open) setDeleting(null)
        }}
      >
        {/* A grid track sized by its content would let a long title widen
            the dialog past its frame. */}
        <AlertDialogContent initialFocus={cancelRef} className="grid-cols-1">
          <AlertDialogHeader>
            <AlertDialogTitle className="wrap-anywhere">
              Delete{" "}
              <bdi>
                {deleting?.title === "" ? "Untitled query" : deleting?.title}
              </bdi>
              ?
            </AlertDialogTitle>
            <AlertDialogDescription>
              The saved query is removed from this workspace. No database object
              is touched.
            </AlertDialogDescription>
          </AlertDialogHeader>
          <AlertDialogFooter className="flex-col sm:flex-row">
            <AlertDialogCancel ref={cancelRef}>Cancel</AlertDialogCancel>
            <Button
              variant="destructive"
              onClick={() => {
                if (deleting) onDeleteSaved(deleting)
                setDeleting(null)
              }}
            >
              Delete query
            </Button>
          </AlertDialogFooter>
        </AlertDialogContent>
      </AlertDialog>

      <AlertDialog
        open={reconciling !== null}
        onOpenChange={(open) => {
          if (!open) setReconciling(null)
        }}
      >
        <AlertDialogContent initialFocus={keepRef} className="grid-cols-1">
          <AlertDialogHeader>
            <AlertDialogTitle className="wrap-anywhere">
              Mark this write on{" "}
              <bdi>
                {reconciling?.connectionName ?? "an unavailable connection"}
              </bdi>{" "}
              reconciled?
            </AlertDialogTitle>
            <AlertDialogDescription>
              Confirm only after inspecting the server state. Launches will stop
              warning about this write. Nothing is sent to the server and the
              write is never retried.
            </AlertDialogDescription>
          </AlertDialogHeader>
          <code className="line-clamp-4 rounded-md border bg-muted p-2 font-mono text-xs break-all">
            {reconciling?.preview}
          </code>
          <AlertDialogFooter className="flex-col sm:flex-row">
            <AlertDialogCancel ref={keepRef}>Keep warning</AlertDialogCancel>
            <Button
              onClick={() => {
                if (reconciling) onReconcile(reconciling)
                setReconciling(null)
              }}
            >
              Mark reconciled
            </Button>
          </AlertDialogFooter>
        </AlertDialogContent>
      </AlertDialog>
    </div>
  )
}
