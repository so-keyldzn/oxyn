import * as React from "react"
import { HugeiconsIcon } from "@hugeicons/react"
import {
  Alert02Icon,
  ArrowLeft01Icon,
  ArrowRight01Icon,
  RefreshIcon,
} from "@hugeicons/core-free-icons"

import { Alert, AlertDescription, AlertTitle } from "@/components/ui/alert"
import {
  Breadcrumb,
  BreadcrumbItem,
  BreadcrumbList,
  BreadcrumbPage,
  BreadcrumbSeparator,
} from "@/components/ui/breadcrumb"
import { Button } from "@/components/ui/button"
import { Checkbox } from "@/components/ui/checkbox"
import { Kbd } from "@/components/ui/kbd"
import { Skeleton } from "@/components/ui/skeleton"
import type { DocumentEntry } from "@/lib/ipc/library"

export type RecoveryState =
  | { status: "loading" }
  | { status: "error"; message: string }
  | { status: "ready"; entries: Array<DocumentEntry> }

/** What the screen may assert: only a crash it observed (ADR-0021). */
function openingNotice(abnormal: boolean, empty: boolean) {
  if (abnormal && empty)
    return "Oxyn did not close normally. No saved working copies were found on this page."
  if (abnormal)
    return "Oxyn did not close normally. Choose the working copies to restore. Connections remain closed."
  if (empty) return "No saved working copies on this page."
  return "Choose the working copies to restore. Connections remain closed."
}

/**
 * The working copies a previous launch left open.
 *
 * Restoring puts text back in consoles and nothing else: no connection opens,
 * no statement runs. The restore action is absent without a selection.
 */
export function RecoveryList({
  abnormal,
  state,
  selected,
  onToggle,
  hasPrevious,
  hasNext,
  onPrevious,
  onNext,
  onRefresh,
  onRestore,
  onContinue,
  backLabel,
  onBack,
}: {
  abnormal: boolean
  state: RecoveryState
  selected: ReadonlySet<string>
  onToggle: (entry: DocumentEntry) => void
  hasPrevious: boolean
  hasNext: boolean
  onPrevious: () => void
  onNext: () => void
  onRefresh: () => void
  onRestore: () => void
  onContinue: () => void
  /** Where Back and Esc go: « Back to workspace » or « Back to connections ». */
  backLabel: string
  onBack: () => void
}) {
  const count = selected.size
  const keptId = React.useId()
  return (
    <div className="mx-auto flex w-full max-w-2xl flex-col gap-4 p-6">
      <header className="flex flex-col gap-1">
        <Breadcrumb>
          <BreadcrumbList className="text-xs">
            <BreadcrumbItem>
              <Button
                size="xs"
                variant="ghost"
                onClick={onBack}
                aria-keyshortcuts="Escape Meta+BracketLeft"
              >
                <HugeiconsIcon
                  icon={ArrowLeft01Icon}
                  strokeWidth={2}
                  data-icon="inline-start"
                />
                {backLabel}
                <Kbd>Esc</Kbd>
              </Button>
            </BreadcrumbItem>
            <BreadcrumbSeparator />
            <BreadcrumbItem>
              <BreadcrumbPage>Recovery</BreadcrumbPage>
            </BreadcrumbItem>
          </BreadcrumbList>
        </Breadcrumb>
        <h1 className="text-xl font-semibold">Recover your workspace</h1>
        <p className="text-sm text-muted-foreground" role="status">
          {state.status === "loading"
            ? "Looking for saved working copies…"
            : state.status === "error"
              ? state.message
              : openingNotice(abnormal, state.entries.length === 0)}
        </p>
      </header>

      <Alert>
        <HugeiconsIcon icon={Alert02Icon} strokeWidth={2} />
        <AlertTitle>A write may have an unknown outcome</AlertTitle>
        <AlertDescription>
          If a write was interrupted, inspect the server state before deciding
          what to do. Recovery never retries it.
        </AlertDescription>
      </Alert>

      {state.status === "loading" ? (
        <div className="flex flex-col gap-2" aria-busy="true">
          <Skeleton className="h-10" />
          <Skeleton className="h-10" />
        </div>
      ) : state.status === "error" ? (
        <Alert variant="destructive">
          <AlertDescription>{state.message}</AlertDescription>
        </Alert>
      ) : state.entries.length === 0 ? null : (
        <ul className="flex flex-col divide-y rounded-md border">
          {state.entries.map((entry) => {
            const title = entry.title === "" ? "Untitled query" : entry.title
            return (
              <li key={entry.id}>
                <label className="flex cursor-pointer items-center gap-3 px-3 py-2 text-sm">
                  <Checkbox
                    checked={selected.has(entry.id)}
                    onCheckedChange={() => onToggle(entry)}
                    aria-label={`Restore ${title}`}
                  />
                  <span className="min-w-0 flex-1 truncate">{title}</span>
                  <span className="truncate text-xs text-muted-foreground">
                    {entry.connectionName ?? "No connection"}
                  </span>
                </label>
              </li>
            )
          })}
        </ul>
      )}

      {/* Wraps: at 420 px the two decisions do not fit beside the pager, and
          without wrapping « Restore » was pushed past the window's edge. */}
      <div className="flex flex-wrap items-center gap-1">
        <Button
          size="icon-sm"
          variant="ghost"
          aria-label="Previous page"
          disabled={!hasPrevious}
          onClick={onPrevious}
        >
          <HugeiconsIcon icon={ArrowLeft01Icon} strokeWidth={2} />
        </Button>
        <Button
          size="icon-sm"
          variant="ghost"
          aria-label="Next page"
          disabled={!hasNext}
          onClick={onNext}
        >
          <HugeiconsIcon icon={ArrowRight01Icon} strokeWidth={2} />
        </Button>
        <Button size="sm" variant="ghost" onClick={onRefresh}>
          <HugeiconsIcon
            icon={RefreshIcon}
            strokeWidth={2}
            data-icon="inline-start"
          />
          Refresh
        </Button>
        <div className="ml-auto flex flex-wrap justify-end gap-2">
          <Button
            variant="outline"
            onClick={onContinue}
            aria-describedby={keptId}
          >
            Skip for now
          </Button>
          {count > 0 ? (
            <Button onClick={onRestore}>
              Restore {count} selected {count === 1 ? "item" : "items"}
            </Button>
          ) : null}
        </div>
      </div>

      <p id={keptId} className="text-xs text-muted-foreground">
        Skipping deletes nothing: working copies stay in the library and can be
        resumed later.
      </p>
      <p className="text-xs text-muted-foreground">
        Offline · Choose a connection before running · Nothing executed
        automatically
      </p>
    </div>
  )
}
