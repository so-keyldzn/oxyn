import { useId, useState } from "react"

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
  DialogTrigger,
} from "@/components/ui/dialog"
import { Field, FieldGroup, FieldLabel } from "@/components/ui/field"

/** MySQL does not enumerate temporary tables; the owner can open one by name. */
export function OpenSessionTable({
  onOpen,
}: {
  onOpen: (database: string, table: string) => void
}) {
  const [open, setOpen] = useState(false)
  const [database, setDatabase] = useState("")
  const [table, setTable] = useState("")
  const id = useId()
  const valid = database.trim() !== "" && table.trim() !== ""
  return (
    <Dialog open={open} onOpenChange={setOpen}>
      <DialogTrigger
        render={<Button variant="outline" size="sm" className="mx-3 mb-3" />}
      >
        Open table by name…
      </DialogTrigger>
      <DialogContent>
        <form
          className="grid gap-4"
          onSubmit={(event) => {
            event.preventDefault()
            if (!valid) return
            onOpen(database, table)
            setOpen(false)
          }}
        >
          <DialogHeader>
            <DialogTitle>Open a table on this session</DialogTitle>
            <DialogDescription>
              MySQL does not list temporary tables in its catalog. Enter the
              names without SQL quotes; the table is read on the selected
              console session.
            </DialogDescription>
          </DialogHeader>
          <FieldGroup>
            <Field>
              <FieldLabel htmlFor={`${id}-database`}>Database</FieldLabel>
              <TextInput
                id={`${id}-database`}
                value={database}
                onChange={(event) => setDatabase(event.target.value)}
                autoFocus
                required
              />
            </Field>
            <Field>
              <FieldLabel htmlFor={`${id}-table`}>Table</FieldLabel>
              <TextInput
                id={`${id}-table`}
                value={table}
                onChange={(event) => setTable(event.target.value)}
                required
              />
            </Field>
          </FieldGroup>
          <DialogFooter>
            <DialogClose render={<Button type="button" variant="outline" />}>
              Cancel
            </DialogClose>
            <Button type="submit" disabled={!valid}>
              Open table
            </Button>
          </DialogFooter>
        </form>
      </DialogContent>
    </Dialog>
  )
}
