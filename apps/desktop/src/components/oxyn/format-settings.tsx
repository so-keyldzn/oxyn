import * as React from "react"

import {
  Field,
  FieldDescription,
  FieldError,
  FieldGroup,
  FieldLabel,
  FieldLegend,
  FieldSet,
} from "@/components/ui/field"
import { NativeSelect, NativeSelectOption } from "@/components/ui/native-select"
import { ToggleGroup, ToggleGroupItem } from "@/components/ui/toggle-group"
import type {
  BinaryChoice,
  DisplayPreferences,
  PreferencesChange,
} from "@/lib/ipc/settings"
import { TextInput } from "./text-field"

// Previews, not explanations: « 4 823 917 » says at a glance what a sentence
// says badly. Written here rather than formatted in JavaScript — the front
// never formats a cell — and pinned to what `oxyn_data::format_cell` renders
// by `backend::settings::tests::the_previews_are_what_the_formatter_renders`.
export const GROUPING_PREVIEW = {
  none: "4823917",
  thousands: "4\u00a0823\u00a0917",
} as const

export const BINARY_PREVIEW: Record<BinaryChoice, string> = {
  hex: "48656c6c6f",
  base64: "SGVsbG8=",
  size: "<5 B>",
}

const BINARY_LABEL: Record<BinaryChoice, string> = {
  hex: "Hex",
  base64: "Base64",
  size: "Size only",
}

/** Truncation lengths offered, inside the 64–16 384 range the domain accepts. */
export const CELL_LENGTHS: ReadonlyArray<number> = [
  128, 256, 512, 2048, 8192, 16384,
]

/** The domain's bound on the absent-value label, in UTF-8 bytes (ADR-0013). */
export const NULL_TEXT_MAX_BYTES = 64

export function utf8Length(text: string) {
  return new TextEncoder().encode(text).length
}

/**
 * How result cells are drawn. Display only: the query, what the server
 * returns and what an export writes are unchanged, and the formatting itself
 * happens in Rust (docs/UX-SPEC.md « Ce qui est exporté est ce qui est
 * affiché »).
 *
 * The absent-value label is emitted as typed; the caller debounces the save.
 */
export function FormatSettings({
  preferences,
  onChange,
}: {
  preferences: DisplayPreferences
  onChange: (change: PreferencesChange) => void
}) {
  const [nullText, setNullText] = React.useState(preferences.nullText)
  React.useEffect(
    () => setNullText(preferences.nullText),
    [preferences.nullText]
  )
  const tooLong = utf8Length(nullText) > NULL_TEXT_MAX_BYTES

  return (
    <FieldGroup>
      <Field data-invalid={tooLong || undefined}>
        <FieldLabel htmlFor="format-null-text">Missing value marker</FieldLabel>
        <TextInput
          id="format-null-text"
          value={nullText}
          aria-invalid={tooLong || undefined}
          onChange={(event) => {
            const next = event.target.value
            setNullText(next)
            // Never sent past the bound: the backend would refuse the whole
            // change, and the display would disagree with what is saved.
            if (utf8Length(next) <= NULL_TEXT_MAX_BYTES)
              onChange({ nullText: next })
          }}
        />
        <FieldDescription>
          Drawn in italics, distinct from a text value that reads « NULL ».
        </FieldDescription>
        <FieldError
          errors={
            tooLong
              ? [{ message: `At most ${NULL_TEXT_MAX_BYTES} bytes.` }]
              : []
          }
        />
      </Field>

      <FieldSet>
        <FieldLegend variant="label">Number format</FieldLegend>
        <FieldDescription>Does not affect exported files.</FieldDescription>
        <ToggleGroup
          variant="outline"
          spacing={0}
          value={[preferences.groupThousands ? "thousands" : "none"]}
          onValueChange={(next: Array<unknown>) => {
            if (next[0] === "thousands" || next[0] === "none")
              onChange({ groupThousands: next[0] === "thousands" })
          }}
          aria-label="Number format"
        >
          <ToggleGroupItem value="none">None</ToggleGroupItem>
          <ToggleGroupItem value="thousands">Thousands</ToggleGroupItem>
        </ToggleGroup>
        <p
          className="font-mono text-xs text-muted-foreground"
          data-testid="grouping-preview"
        >
          {preferences.groupThousands
            ? GROUPING_PREVIEW.thousands
            : GROUPING_PREVIEW.none}
        </p>
      </FieldSet>

      <FieldSet>
        <FieldLegend variant="label">Binary values</FieldLegend>
        <FieldDescription>
          « Size only » keeps a column of images or documents readable.
        </FieldDescription>
        <ToggleGroup
          variant="outline"
          spacing={0}
          value={[preferences.binaryDisplay]}
          onValueChange={(next: Array<unknown>) => {
            const chosen = next[0]
            if (chosen === "hex" || chosen === "base64" || chosen === "size")
              onChange({ binaryDisplay: chosen })
          }}
          aria-label="Binary values"
        >
          {(Object.keys(BINARY_LABEL) as Array<BinaryChoice>).map((choice) => (
            <ToggleGroupItem key={choice} value={choice}>
              {BINARY_LABEL[choice]}
            </ToggleGroupItem>
          ))}
        </ToggleGroup>
        <p
          className="font-mono text-xs text-muted-foreground"
          data-testid="binary-preview"
        >
          {BINARY_PREVIEW[preferences.binaryDisplay]}
        </p>
      </FieldSet>

      <Field>
        <FieldLabel htmlFor="format-cell-length">Cell length</FieldLabel>
        <NativeSelect
          id="format-cell-length"
          value={String(preferences.cellMaxChars)}
          onChange={(event) =>
            onChange({ cellMaxChars: Number(event.target.value) })
          }
        >
          {(CELL_LENGTHS.includes(preferences.cellMaxChars)
            ? CELL_LENGTHS
            : [...CELL_LENGTHS, preferences.cellMaxChars].sort((a, b) => a - b)
          ).map((length) => (
            <NativeSelectOption key={length} value={String(length)}>
              {length.toLocaleString("en-US")} characters
            </NativeSelectOption>
          ))}
        </NativeSelect>
        <FieldDescription>
          A longer value is cut in the grid; the value inspector shows it whole.
          Timestamps stay in RFC 3339, the format exports write.
        </FieldDescription>
      </Field>
    </FieldGroup>
  )
}
