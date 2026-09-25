import * as React from "react"

import {
  AlertDialog,
  AlertDialogAction,
  AlertDialogCancel,
  AlertDialogContent,
  AlertDialogDescription,
  AlertDialogFooter,
  AlertDialogHeader,
  AlertDialogTitle,
} from "@/components/ui/alert-dialog"

/**
 * Leaving a form that holds typed values. « Keep editing » takes the focus,
 * so a reflex Enter or Esc loses nothing.
 */
export function DiscardChangesDialog({
  open,
  what,
  onKeep,
  onDiscard,
}: {
  open: boolean
  /** What is abandoned, e.g. « this new connection ». */
  what: string
  onKeep: () => void
  onDiscard: () => void
}) {
  const keepRef = React.useRef<HTMLButtonElement>(null)
  return (
    <AlertDialog
      open={open}
      onOpenChange={(next) => {
        if (!next) onKeep()
      }}
    >
      {/* A grid track sized by its content would let a long subject widen
          the dialog past its frame. */}
      <AlertDialogContent initialFocus={keepRef} className="grid-cols-1">
        <AlertDialogHeader>
          <AlertDialogTitle className="wrap-anywhere">
            Discard {what}?
          </AlertDialogTitle>
          <AlertDialogDescription>
            The details you typed are not saved anywhere and will be lost.
          </AlertDialogDescription>
        </AlertDialogHeader>
        <AlertDialogFooter>
          <AlertDialogCancel ref={keepRef}>Keep editing</AlertDialogCancel>
          <AlertDialogAction variant="destructive" onClick={onDiscard}>
            Discard
          </AlertDialogAction>
        </AlertDialogFooter>
      </AlertDialogContent>
    </AlertDialog>
  )
}
