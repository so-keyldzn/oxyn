import { useId } from "react"
import { Field, FieldDescription, FieldLabel } from "@/components/ui/field"
import { NativeSelect, NativeSelectOption } from "@/components/ui/native-select"

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
  return (
    <Field className="p-3">
      <FieldLabel htmlFor={id}>Browse session</FieldLabel>
      <NativeSelect
        id={id}
        value={value}
        onChange={(event) => onChange(event.target.value)}
      >
        <NativeSelectOption value="">Connection catalog</NativeSelectOption>
        {choices.map((choice) => (
          <NativeSelectOption key={choice.id} value={choice.id}>
            {choice.label}
          </NativeSelectOption>
        ))}
      </NativeSelect>
      {value !== "" && (
        <FieldDescription>
          Objects use this console’s session. If a data preview occupies it,
          close the data tab before running another query.
        </FieldDescription>
      )}
    </Field>
  )
}
