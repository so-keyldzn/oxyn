import { Button } from "@/components/ui/button"
import {
  Dialog,
  DialogContent,
  DialogHeader,
  DialogTitle,
  DialogDescription,
  DialogFooter,
} from "@/components/ui/dialog"

/** A foreign editor never receives a name before its source is reviewed. */
export function CatalogTransferDialog({
  name,
  connection,
  onSource,
  onCancel,
}: {
  name: string
  connection: string
  onSource: () => void
  onCancel: () => void
}) {
  return (
    <Dialog
      open
      onOpenChange={(open) => {
        if (!open) onCancel()
      }}
    >
      <DialogContent>
        <DialogHeader>
          <DialogTitle>Choose the source console</DialogTitle>
          <DialogDescription>
            {name} belongs to {connection}. The destination editor uses another
            connection or private session. Nothing was inserted.
          </DialogDescription>
        </DialogHeader>
        <DialogFooter>
          <Button autoFocus variant="outline" onClick={onCancel}>
            Cancel
          </Button>
          <Button onClick={onSource}>Insert in source console</Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  )
}
