import * as React from "react"

import {
  AlertDialog,
  AlertDialogCancel,
  AlertDialogContent,
  AlertDialogDescription,
  AlertDialogFooter,
  AlertDialogHeader,
  AlertDialogTitle,
} from "@/components/ui/alert-dialog"
import { Button } from "@/components/ui/button"
import { Spinner } from "@/components/ui/spinner"
import { EnvironmentBadge } from "@/components/oxyn/environment-badge"
import type { Environment, TransactionState } from "@/lib/ipc/types"

/** One console whose transaction holds the exit. */
export interface ExitTransactionRow {
  session: string
  /** The console's title; the backend knows sessions, the front names them. */
  console: string
  connectionName: string
  environment: Environment
  state: TransactionState
}

const STATE_LABEL: Record<TransactionState, string> = {
  open: "Transaction open",
  unknown: "Transaction state unknown",
  idle: "No transaction",
}

/**
 * The decision an exit needs while consoles hold a transaction (ADR-0043):
 * commit or roll back every listed one, or cancel the exit.
 *
 * Each console is named with its connection and environment. `Cancel` takes
 * the focus and Enter on the body does nothing: neither committing nor
 * throwing writes away is a reflex answer. A failure shows the server's
 * message and the dialog stays open; `Cancel` stays available while a
 * statement runs.
 */
export function ExitTransactionsDialog({
  rows,
  busy = null,
  error = null,
  commitBlocked = false,
  onCommit,
  onRollback,
  onCancel,
}: {
  /** `null` keeps the dialog closed. */
  rows: ReadonlyArray<ExitTransactionRow> | null
  busy?: "commit" | "rollback" | null
  /** The last failure, as the server said it. */
  error?: string | null
  /**
   * Every listed `COMMIT` already failed or ended without an answer: it is
   * not sent again from here (I-13).
   */
  commitBlocked?: boolean
  onCommit: () => void
  onRollback: () => void
  onCancel: () => void
}) {
  const cancelRef = React.useRef<HTMLButtonElement>(null)
  const count = rows?.length ?? 0
  const nothing = count === 0
  return (
    <AlertDialog
      open={rows !== null}
      onOpenChange={(open) => {
        if (!open) onCancel()
      }}
    >
      <AlertDialogContent
        className="grid-cols-1 data-[size=default]:sm:max-w-lg"
        initialFocus={cancelRef}
        onKeyDown={(event) => {
          if (
            event.key === "Enter" &&
            !(event.target instanceof HTMLButtonElement)
          ) {
            event.preventDefault()
          }
        }}
      >
        <AlertDialogHeader>
          <AlertDialogTitle>
            {count === 1
              ? "Quit with an open transaction?"
              : "Quit with open transactions?"}
          </AlertDialogTitle>
          <AlertDialogDescription>
            {nothing
              ? "No transaction is open any more. Cancel, then quit again."
              : "Commit or roll back to quit. Cancel keeps Oxyn open, with every transaction as it is."}
          </AlertDialogDescription>
        </AlertDialogHeader>
        {nothing ? null : (
          <ul
            aria-label="Open transactions"
            className="flex max-h-64 flex-col gap-2 overflow-y-auto"
          >
            {rows?.map((row) => (
              <li
                key={row.session}
                className="flex flex-col gap-1 rounded-md border px-3 py-2 text-sm"
              >
                <span className="font-medium wrap-anywhere">{row.console}</span>
                <span className="flex flex-wrap items-center gap-2 text-muted-foreground">
                  <span className="wrap-anywhere">{row.connectionName}</span>
                  <EnvironmentBadge environment={row.environment} />
                  <span>{STATE_LABEL[row.state]}</span>
                </span>
              </li>
            ))}
          </ul>
        )}
        {error ? (
          <p className="text-xs wrap-anywhere text-destructive" role="alert">
            {error}
          </p>
        ) : null}
        <AlertDialogFooter className="flex-col sm:flex-row">
          <AlertDialogCancel ref={cancelRef}>Cancel</AlertDialogCancel>
          <Button
            variant="destructive"
            disabled={nothing || busy !== null}
            onClick={onRollback}
          >
            {busy === "rollback" ? <Spinner data-icon="inline-start" /> : null}
            Rollback
          </Button>
          <Button
            disabled={nothing || commitBlocked || busy !== null}
            onClick={onCommit}
          >
            {busy === "commit" ? <Spinner data-icon="inline-start" /> : null}
            Commit
          </Button>
        </AlertDialogFooter>
      </AlertDialogContent>
    </AlertDialog>
  )
}
