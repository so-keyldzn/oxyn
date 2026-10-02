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

import { busyLines } from "./update-model"

/**
 * `Restart now` while work runs (ADR-0051, docs/UX-SPEC.md « Updates »): the
 * running queries and exports are named, and what stopping them does.
 *
 * `Later` takes the focus and Enter on the body does nothing: an update can
 * wait, a query cancelled on the server cannot be taken back. Nothing running
 * means no dialog at all — the host does not open it.
 */
export function RestartToUpdateDialog({
  work,
  onLater,
  onStopAndRestart,
}: {
  /** `null` keeps the dialog closed. */
  work: { running: number; exports: number } | null
  onLater: () => void
  onStopAndRestart: () => void
}) {
  const laterRef = React.useRef<HTMLButtonElement>(null)
  return (
    <AlertDialog
      open={work !== null}
      onOpenChange={(open) => {
        if (!open) onLater()
      }}
    >
      <AlertDialogContent
        className="grid-cols-1 data-[size=default]:sm:max-w-lg"
        initialFocus={laterRef}
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
            Restart now and stop running work?
          </AlertDialogTitle>
          <AlertDialogDescription render={<div />}>
            {work
              ? busyLines(work.running, work.exports).map((line) => (
                  <p key={line}>{line}</p>
                ))
              : null}
            <p>Your consoles come back in their windows, offline.</p>
          </AlertDialogDescription>
        </AlertDialogHeader>
        <AlertDialogFooter className="flex-col sm:flex-row">
          <AlertDialogCancel ref={laterRef}>Later</AlertDialogCancel>
          <Button variant="destructive" onClick={onStopAndRestart}>
            Stop and restart
          </Button>
        </AlertDialogFooter>
      </AlertDialogContent>
    </AlertDialog>
  )
}
