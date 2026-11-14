import { useId } from "react"
import { Field, FieldDescription, FieldLabel } from "@/components/ui/field"
import {
  Select,
  SelectContent,
  SelectGroup,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select"

/** A console's temporary objects remain attached to that console's session. */
export function CatalogSource({
  value,
  choices,
  onChange,
}: {
  value: string
  choices: Array<{ id: string; label: string }>
  onChange: (id: string) => void
}) {
  const id = useId()
  const items = [
    { value: "", label: "Connection catalog" },
    ...choices.map((choice) => ({ value: choice.id, label: choice.label })),
  ]
  return (
    <Field className="p-3">
      <FieldLabel htmlFor={id}>Browse session</FieldLabel>
      <Select
        items={items}
        value={value}
        onValueChange={(next) => {
          if (typeof next === "string") onChange(next)
        }}
      >
        <SelectTrigger id={id} aria-label="Browse session" className="w-full">
          <SelectValue />
        </SelectTrigger>
        <SelectContent alignItemWithTrigger={false}>
          <SelectGroup>
            {items.map((item) => (
              <SelectItem key={item.value} value={item.value}>
                {item.label}
              </SelectItem>
            ))}
          </SelectGroup>
        </SelectContent>
      </Select>
      {value !== "" && (
        <FieldDescription>
          Objects use this console’s session. If a data preview occupies it,
          close the data tab before running another query.
        </FieldDescription>
      )}
    </Field>
  )
}
