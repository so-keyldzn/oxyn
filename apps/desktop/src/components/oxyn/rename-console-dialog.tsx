import * as React from "react"

import { TextInput } from "@/components/oxyn/text-field"
import { Button } from "@/components/ui/button"
import {
  Dialog,
  DialogClose,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog"
import { Field, FieldError, FieldLabel } from "@/components/ui/field"
import { titleTooLong } from "@/features/consoles/console-model"

/** Why a name is refused, or null: the rules of a saved query's name. */
function refusal(title: string) {
  if (title.trim() === "") return "A query needs a name."
  if (titleTooLong(title)) return "Query names must not exceed 256 UTF-8 bytes."
  return null
}

/**
 * `Rename…` of a console tab: the name its « Query name » field holds,
 * changed from the tab. The console then shows it unsaved, as a name typed
 * in the field would; nothing is written to the library from here.
 */
export function RenameConsoleDialog({
  title,
  onCancel,
  onRename,
}: {
  /** The current name; `null` keeps the dialog closed. */
  title: string | null
  onCancel: () => void
  onRename: (title: string) => void
}) {
  const [value, setValue] = React.useState(title ?? "")
  // Each opening starts from the console's name, not from the last edit.
  const [opened, setOpened] = React.useState(title)
  if (opened !== title) {
    setOpened(title)
    setValue(title ?? "")
  }
  const problem = refusal(value)
  const fieldId = React.useId()
  const errorId = React.useId()
  const submit = () => {
    if (problem === null) onRename(value)
  }
  return (
    <Dialog
      open={title !== null}
      onOpenChange={(open) => {
        if (!open) onCancel()
      }}
    >
      <DialogContent>
        <form
          className="grid gap-4"
          onSubmit={(event) => {
            event.preventDefault()
            submit()
          }}
        >
          <DialogHeader>
            <DialogTitle className="wrap-anywhere">Rename {title}</DialogTitle>
            <DialogDescription>
              The console keeps its text and its session. Save it to keep the
              new name in the library.
            </DialogDescription>
          </DialogHeader>
          <Field data-invalid={problem !== null || undefined}>
            <FieldLabel htmlFor={fieldId}>Query name</FieldLabel>
            <TextInput
              id={fieldId}
              value={value}
              dir="auto"
              autoFocus
              onChange={(event) => setValue(event.target.value)}
              aria-invalid={problem !== null}
              aria-describedby={problem ? errorId : undefined}
            />
            <FieldError id={errorId}>{problem}</FieldError>
          </Field>
          <DialogFooter>
            <DialogClose render={<Button type="button" variant="outline" />}>
              Cancel
            </DialogClose>
            <Button type="submit" disabled={problem !== null}>
              Rename
            </Button>
          </DialogFooter>
        </form>
      </DialogContent>
    </Dialog>
  )
}
