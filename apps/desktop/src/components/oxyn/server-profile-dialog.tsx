import * as React from "react"
import { Button } from "@/components/ui/button"
import { Field, FieldLabel, FieldGroup } from "@/components/ui/field"
import {
  Dialog,
  DialogContent,
  DialogHeader,
  DialogTitle,
  DialogDescription,
} from "@/components/ui/dialog"
import { TextInput } from "./text-field"
import { BackendErrorAlert } from "./backend-error-alert"
import type { BackendFailure } from "./backend-error-alert"

export function ServerProfileDialog({
  name,
  removing,
  renaming = false,
  busy,
  error,
  onSave,
  onClose,
}: {
  name: string
  renaming?: boolean
  removing: boolean
  busy: boolean
  error: BackendFailure | null
  onSave: (name: string) => void
  onClose: () => void
}) {
  const [value, setValue] = React.useState(name)
  const id = React.useId()
  return (
    <Dialog
      open
      onOpenChange={(open) => {
        if (!open && !busy) onClose()
      }}
    >
      <DialogContent showCloseButton={!busy}>
        <DialogHeader>
          <DialogTitle>
            {removing
              ? "Remove server group?"
              : renaming
                ? "Rename server group"
                : "Group the connected server"}
          </DialogTitle>
          <DialogDescription>
            {removing
              ? "Saved connections, credentials and open workspaces stay intact. Only the group is removed."
              : "The active PostgreSQL connection becomes the discovery source. Other databases keep independent accounts and protections."}
          </DialogDescription>
        </DialogHeader>
        <form
          className="flex flex-col gap-4"
          onSubmit={(e) => {
            e.preventDefault()
            onSave(value)
          }}
        >
          {!removing && (
            <FieldGroup>
              <Field>
                <FieldLabel htmlFor={id}>Group name</FieldLabel>
                <TextInput
                  id={id}
                  autoFocus
                  value={value}
                  onChange={(e) => setValue(e.target.value)}
                  disabled={busy}
                  required
                />
              </Field>
            </FieldGroup>
          )}
          {error && (
            <BackendErrorAlert title="Group not changed" error={error} />
          )}
          <div className="flex justify-end gap-2">
            <Button
              type="button"
              variant="outline"
              disabled={busy}
              onClick={onClose}
              autoFocus={removing}
            >
              Cancel
            </Button>
            <Button
              type="submit"
              variant={removing ? "destructive" : "default"}
              disabled={busy || (!removing && !value.trim())}
            >
              {busy
                ? "Saving…"
                : removing
                  ? "Remove group"
                  : renaming
                    ? "Rename group"
                    : "Create group"}
            </Button>
          </div>
        </form>
      </DialogContent>
    </Dialog>
  )
}
