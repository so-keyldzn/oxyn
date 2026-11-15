import {
  AlertDialog,
  AlertDialogContent,
  AlertDialogHeader,
  AlertDialogTitle,
  AlertDialogDescription,
  AlertDialogFooter,
  AlertDialogCancel,
  AlertDialogAction,
} from "@/components/ui/alert-dialog"
import type { NativeJob } from "@/lib/ipc/native-transfers"

/** A human declaration never changes an unknown outcome into success or rollback. */
export function NativeTransferReconcile({
  job,
  onInspect,
  onCancel,
}: {
  job: NativeJob
  onInspect: () => void
  onCancel: () => void
}) {
  return (
    <AlertDialog
      open
      onOpenChange={(open) => {
        if (!open) onCancel()
      }}
    >
      <AlertDialogContent>
        <AlertDialogHeader>
          <AlertDialogTitle>Record destination inspection?</AlertDialogTitle>
          <AlertDialogDescription>
            You confirm that you inspected <bdi>{job.database}</bdi> on{" "}
            <bdi>{job.connectionName}</bdi> after its uncertain restore. This
            records your inspection; the original outcome stays unchanged and
            nothing is retried.
          </AlertDialogDescription>
        </AlertDialogHeader>
        <AlertDialogFooter>
          <AlertDialogCancel autoFocus onClick={onCancel}>
            Cancel
          </AlertDialogCancel>
          <AlertDialogAction onClick={onInspect}>
            Record my inspection
          </AlertDialogAction>
        </AlertDialogFooter>
      </AlertDialogContent>
    </AlertDialog>
  )
}
