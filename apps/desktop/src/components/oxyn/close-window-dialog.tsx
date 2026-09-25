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

/** A console that would lose work if its window closed. */
export interface CloseWindowRow {
  /** Stable within the dialog; never shown. */
  key: string
  title: string
  /** The connection's name, as its workspace shows it. */
  connection: string
  unsaved: boolean
  /** The stored document changed elsewhere and is left untouched. */
  conflict: boolean
  running: boolean
}

function costs(row: CloseWindowRow) {
  const parts: Array<string> = []
  if (row.unsaved) parts.push("Unsaved SQL")
  if (row.conflict) parts.push("Changed elsewhere")
  if (row.running) parts.push("Operation running")
  return parts.join(" · ")
}

/**
 * The decision the close of a window needs when it is not the last one
 * (ADR-0043): closing it closes its consoles, and those that would lose text
 * or stop an operation are listed in one dialog, not one each.
 *
 * The window is named by the connections it shows: a window has no name of
 * its own. `Cancel` takes the focus and Enter on the body does nothing:
 * discarding work is never the reflex answer.
 */
export function CloseWindowDialog({
  rows,
  connections,
  busy = false,
  onCancel,
  onClose,
}: {
  /** `null` keeps the dialog closed. */
  rows: ReadonlyArray<CloseWindowRow> | null
  /** The connections of the window's workspaces, shown or hidden. */
  connections: ReadonlyArray<string>
  /** The consoles are closing: only Cancel of the wait remains. */
  busy?: boolean
  onCancel: () => void
  onClose: () => void
}) {
  const cancelRef = React.useRef<HTMLButtonElement>(null)
  const named =
    connections.length === 0
      ? "this window"
      : `the window of ${connections.join(", ")}`
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
          <AlertDialogTitle className="wrap-anywhere">
            Close {named}?
          </AlertDialogTitle>
          <AlertDialogDescription>
            Closing this window closes its consoles.{" "}
            {rows?.length === 1
              ? "This one would lose work:"
              : "These would lose work:"}{" "}
            unsaved SQL is discarded and running operations are cancelled.
            Committed database changes are not undone.
          </AlertDialogDescription>
        </AlertDialogHeader>
        <ul
          aria-label="Consoles that would lose work"
          className="flex max-h-64 flex-col gap-2 overflow-y-auto"
        >
          {rows?.map((row) => (
            <li
              key={row.key}
              className="flex flex-col gap-1 rounded-md border px-3 py-2 text-sm"
            >
              <span className="font-medium wrap-anywhere">{row.title}</span>
              <span className="flex flex-wrap items-center gap-2 text-muted-foreground">
                <span className="wrap-anywhere">{row.connection}</span>
                <span>{costs(row)}</span>
              </span>
            </li>
          ))}
        </ul>
        <AlertDialogFooter className="flex-col sm:flex-row">
          <AlertDialogCancel ref={cancelRef}>Cancel</AlertDialogCancel>
          <Button variant="destructive" disabled={busy} onClick={onClose}>
            {busy ? <Spinner data-icon="inline-start" /> : null}
            Discard and close window
          </Button>
        </AlertDialogFooter>
      </AlertDialogContent>
    </AlertDialog>
  )
}
