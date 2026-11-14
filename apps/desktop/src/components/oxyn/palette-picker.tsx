import { FieldDescription, FieldLegend, FieldSet } from "@/components/ui/field"
import { ToggleGroup, ToggleGroupItem } from "@/components/ui/toggle-group"
import { PaletteChoice } from "@/lib/ipc/settings"

export const PALETTE_LABELS: Record<PaletteChoice, string> = {
  oxyn: "Oxyn",
  graphite: "Graphite",
  ocean: "Ocean",
  forest: "Forest",
  plum: "Plum",
}

/** A light and a dark selection are saved independently of the mode switch. */
export function PalettePicker({
  mode,
  value,
  disabled = false,
  onChange,
}: {
  mode: "light" | "dark"
  value: PaletteChoice
  disabled?: boolean
  onChange: (palette: PaletteChoice) => void
}) {
  const label = mode === "light" ? "Light palette" : "Dark palette"
  return (
    <FieldSet data-disabled={disabled || undefined}>
      <FieldLegend variant="label">{label}</FieldLegend>
      <FieldDescription>
        {mode === "light"
          ? "Used in light mode, including when System switches to light."
          : "Used in dark mode, including when System switches to dark."}
      </FieldDescription>
      <ToggleGroup
        variant="outline"
        spacing={2}
        disabled={disabled}
        value={[value]}
        aria-label={label}
        className="grid w-full grid-cols-2 sm:grid-cols-3 xl:grid-cols-5"
        onValueChange={(next: Array<unknown>) => {
          const chosen = PaletteChoice.safeParse(next[0])
          if (chosen.success) onChange(chosen.data)
        }}
      >
        {PaletteChoice.options.map((palette) => (
          <ToggleGroupItem
            key={palette}
            value={palette}
            aria-label={PALETTE_LABELS[palette]}
            className="h-10 justify-start"
          >
            <span
              aria-hidden
              data-palette={palette}
              className="size-3 shrink-0 rounded-full bg-palette-swatch"
            />
            {PALETTE_LABELS[palette]}
          </ToggleGroupItem>
        ))}
      </ToggleGroup>
    </FieldSet>
  )
}
