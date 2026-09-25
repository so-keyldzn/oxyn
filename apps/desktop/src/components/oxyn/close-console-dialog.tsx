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
import { closeMessage } from "@/features/consoles/console-model"
import type { CloseReasons } from "@/features/consoles/console-model"

/**
 * The decision a console close needs when it would lose text, stop a
 * statement (ADR-0015), or roll back a transaction its session reported open
 * or could not rule out (ADR-0039) — the message then names the connection.
 *
 * `Cancel` takes the focus and Enter on the body does nothing: discarding
 * work is never the reflex answer. In conflict the save writes a new copy and
 * leaves the stored document untouched.
 */
export function CloseConsoleDialog({
  reasons,
  busy = false,
  notice,
  onCancel,
  onSave,
  onDiscard,
}: {
  /** `null` keeps the dialog closed. */
  reasons: CloseReasons | null
  /**
   * A save or close is under way: only Cancel stays available, and it cancels
   * that write — its late answer then closes nothing.
   */
  busy?: boolean
  /** What the last save attempt said, shown under the message. */
  notice?: string | null
  onCancel: () => void
  onSave: () => void
  onDiscard: () => void
}) {
  const cancelRef = React.useRef<HTMLButtonElement>(null)
  return (
    <AlertDialog
      open={reasons !== null}
      onOpenChange={(open) => {
        if (!open) onCancel()
      }}
    >
      <AlertDialogContent
        // Three actions side by side need more than the base width, which is
        // keyed on `data-size`; and a grid track sized by its content would
        // let a long file name widen the dialog past its frame.
        className="grid-cols-1 data-[size=default]:sm:max-w-md"
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
            Close {reasons?.title}?
          </AlertDialogTitle>
          <AlertDialogDescription>
            {reasons ? closeMessage(reasons) : null}
          </AlertDialogDescription>
        </AlertDialogHeader>
        {notice ? (
          <p
            className="text-xs wrap-anywhere text-muted-foreground"
            role="status"
          >
            {notice}
          </p>
        ) : null}
        <AlertDialogFooter className="flex-col sm:flex-row">
          <AlertDialogCancel ref={cancelRef}>Cancel</AlertDialogCancel>
          <Button variant="outline" disabled={busy} onClick={onSave}>
            {busy ? <Spinner data-icon="inline-start" /> : null}
            {reasons?.conflict ? "Save copy and close" : "Save and close"}
          </Button>
          <Button variant="destructive" disabled={busy} onClick={onDiscard}>
            Discard and close
          </Button>
        </AlertDialogFooter>
      </AlertDialogContent>
    </AlertDialog>
  )
}
