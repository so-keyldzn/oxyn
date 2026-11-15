import { Button } from "@/components/ui/button"
import {
  Dialog,
  DialogContent,
  DialogHeader,
  DialogTitle,
  DialogDescription,
  DialogFooter,
} from "@/components/ui/dialog"

export interface GroupDisconnectMember {
  id: string
  name: string
  consoles: number
  unsaved: number
  running: number
  transaction: string | null
}

/** Snapshot review; every child still uses its ordinary close safeguards. */
export function GroupDisconnectDialog({
  name,
  members,
  onReview,
  onConfirm,
  onCancel,
}: {
  name: string
  members: Array<GroupDisconnectMember>
  onReview: (id: string) => void
  onConfirm: () => void
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
          <DialogTitle>Disconnect {name}?</DialogTitle>
          <DialogDescription>
            Review every open workspace below. Drafts are reviewed separately
            for each connection. Running operations and unresolved transactions
            must be resolved first. No commit or rollback is automatic.
          </DialogDescription>
        </DialogHeader>
        <ul className="flex max-h-80 flex-col gap-3 overflow-y-auto">
          {members.map((member) => (
            <li key={member.id} className="flex flex-col gap-1">
              <bdi>{member.name}</bdi>
              <p className="text-sm text-muted-foreground">
                {member.consoles} consoles · {member.unsaved} unsaved or
                conflicted · {member.running} operations ·{" "}
                {member.transaction ?? "No pending transaction reported"}
              </p>
              <Button variant="outline" onClick={() => onReview(member.id)}>
                Review workspace · {member.name}
              </Button>
            </li>
          ))}
        </ul>
        {members.length === 0 && <p>No open connections in this group.</p>}
        <DialogFooter>
          <Button autoFocus variant="outline" onClick={onCancel}>
            Cancel
          </Button>
          <Button
            variant="destructive"
            disabled={
              members.length === 0 ||
              members.some(
                (member) => member.running > 0 || member.transaction !== null
              )
            }
            onClick={onConfirm}
          >
            Continue to draft reviews
          </Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  )
}
