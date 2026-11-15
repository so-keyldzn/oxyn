import { Button } from "@/components/ui/button"
import {
  AlertDialog,
  AlertDialogContent,
  AlertDialogHeader,
  AlertDialogTitle,
  AlertDialogDescription,
  AlertDialogFooter,
  AlertDialogCancel,
} from "@/components/ui/alert-dialog"

/** Closing waits for owned cleanup; a restore's stop never promises rollback. */
export function NativeTransferStopReview({
  connection,
  restoring,
  stopping,
  onStop,
  onKeep,
}: {
  connection: string
  restoring: boolean
  stopping: boolean
  onStop: () => void
  onKeep: () => void
}) {
  return (
    <AlertDialog
      open
      onOpenChange={(open) => {
        if (!open && !stopping) onKeep()
      }}
    >
      <AlertDialogContent>
        <AlertDialogHeader>
          <AlertDialogTitle>
            Cancel this transfer before closing?
          </AlertDialogTitle>
          <AlertDialogDescription>
            The native operation on <bdi>{connection}</bdi> stays owned until
            its processes and files finish cleanup.{" "}
            {restoring
              ? "The destination may already have changed. Cancellation does not prove rollback; inspect an uncertain restore before starting a new one."
              : "An interrupted partial export is not published as completed."}
          </AlertDialogDescription>
        </AlertDialogHeader>
        <AlertDialogFooter>
          <AlertDialogCancel autoFocus disabled={stopping} onClick={onKeep}>
            Keep transfer open
          </AlertDialogCancel>
          <Button
            variant="destructive"
            disabled={stopping}
            onClick={(event) => {
              event.preventDefault()
              onStop()
            }}
          >
            {stopping ? "Waiting for cleanup…" : "Cancel transfer and close"}
          </Button>
        </AlertDialogFooter>
      </AlertDialogContent>
    </AlertDialog>
  )
}
