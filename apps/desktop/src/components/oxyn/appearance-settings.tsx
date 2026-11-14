import { ScrollArea } from "@/components/ui/scroll-area"
import { HugeiconsIcon } from "@hugeicons/react"
import { Alert02Icon, CheckmarkCircle02Icon } from "@hugeicons/core-free-icons"

import { Button } from "@/components/ui/button"
import { PalettePicker } from "@/components/oxyn/palette-picker"
import {
  FieldDescription,
  FieldGroup,
  FieldLegend,
  FieldSet,
  FieldSeparator,
} from "@/components/ui/field"
import { Spinner } from "@/components/ui/spinner"
import { ToggleGroup, ToggleGroupItem } from "@/components/ui/toggle-group"
import type {
  DensityChoice,
  DisplayPreferences,
  PreferencesChange,
  ThemeChoice,
} from "@/lib/ipc/settings"

const THEMES: ReadonlyArray<{ value: ThemeChoice; label: string }> = [
  { value: "light", label: "Light" },
  { value: "dark", label: "Dark" },
  { value: "system", label: "System" },
]

/** The two reading presets (docs/UX-SPEC.md « Readability and grid height »). */
export const DENSITIES: ReadonlyArray<{
  value: DensityChoice
  label: string
  detail: string
}> = [
  { value: "compact", label: "Compact", detail: "13 px text · 24 px rows" },
  {
    value: "comfortable",
    label: "Comfortable",
    detail: "14 px text · 28 px rows",
  },
]

/**
 * Palette and reading comfort. Independent of each other and of the window
 * width, and neither reruns a query (docs/UX-SPEC.md « Readability and grid
 * height »).
 */
export function AppearanceSettings({
  preferences,
  palettesAvailable = true,
  onChange,
}: {
  preferences: DisplayPreferences
  palettesAvailable?: boolean
  onChange: (change: PreferencesChange) => void
}) {
  return (
    <FieldGroup>
      <FieldSet>
        <FieldLegend variant="label">Mode</FieldLegend>
        <FieldDescription>
          « System » follows the operating system as it changes.
        </FieldDescription>
        <ToggleGroup
          variant="outline"
          spacing={2}
          className="w-full"
          value={[preferences.theme]}
          onValueChange={(next: Array<unknown>) => {
            const chosen = THEMES.find((theme) => theme.value === next[0])
            if (chosen) onChange({ theme: chosen.value })
          }}
          aria-label="Theme"
        >
          {THEMES.map((theme) => (
            <ToggleGroupItem
              key={theme.value}
              value={theme.value}
              className="h-11 flex-1"
            >
              {theme.label}
            </ToggleGroupItem>
          ))}
        </ToggleGroup>
      </FieldSet>
      <FieldSeparator />

      <PalettePicker
        mode="light"
        value={preferences.lightPalette}
        disabled={!palettesAvailable}
        onChange={(lightPalette) => onChange({ lightPalette })}
      />
      <PalettePicker
        mode="dark"
        value={preferences.darkPalette}
        disabled={!palettesAvailable}
        onChange={(darkPalette) => onChange({ darkPalette })}
      />
      {!palettesAvailable ? (
        <FieldDescription>
          Color palettes are not available in this desktop build.
        </FieldDescription>
      ) : null}
      <FieldSeparator />
      <FieldSet>
        <FieldLegend variant="label">Reading comfort</FieldLegend>
        <FieldDescription>Applies to text and result rows.</FieldDescription>
        <ToggleGroup
          variant="outline"
          spacing={2}
          className="w-full"
          value={[preferences.readingDensity]}
          onValueChange={(next: Array<unknown>) => {
            const chosen = DENSITIES.find(
              (density) => density.value === next[0]
            )
            if (chosen) onChange({ readingDensity: chosen.value })
          }}
          aria-label="Reading comfort"
        >
          {DENSITIES.map((density) => (
            <ToggleGroupItem
              key={density.value}
              value={density.value}
              aria-label={`${density.label}, ${density.detail}`}
              className="h-11 flex-1"
            >
              {density.label}
            </ToggleGroupItem>
          ))}
        </ToggleGroup>
        <p className="text-xs text-muted-foreground">
          {
            DENSITIES.find(
              (density) => density.value === preferences.readingDensity
            )?.detail
          }
        </p>
      </FieldSet>
    </FieldGroup>
  )
}

export type SaveStatus =
  | { status: "idle" }
  | { status: "saving" }
  | { status: "saved" }
  | { status: "failed"; message: string }

/**
 * Whether preferences reached the workspace. A failure keeps the choice
 * applied, says it is not saved and offers to save again — it never reads as
 * saved (ADR-0013).
 */
export function PreferencesSaveStatus({
  save,
  onRetry,
}: {
  save: SaveStatus
  onRetry: () => void
}) {
  return (
    <div
      role="status"
      aria-live="polite"
      className="flex min-h-7 flex-wrap items-center gap-x-2 gap-y-1.5 text-xs text-muted-foreground"
    >
      {save.status === "saving" ? (
        <>
          <Spinner className="size-3" aria-hidden />
          Saving preferences…
        </>
      ) : save.status === "saved" ? (
        <>
          <HugeiconsIcon
            icon={CheckmarkCircle02Icon}
            strokeWidth={2}
            className="size-3.5 text-success"
            aria-hidden
          />
          Preferences saved in this workspace.
        </>
      ) : save.status === "failed" ? (
        <>
          <HugeiconsIcon
            icon={Alert02Icon}
            strokeWidth={2}
            className="size-3.5 text-warning"
            aria-hidden
          />
          <ScrollArea className="max-h-24 min-w-0 flex-1 basis-40 [&>[data-slot=scroll-area-viewport]]:max-h-24">
            <span
              data-selectable
              tabIndex={0}
              className="wrap-anywhere text-foreground outline-none focus-visible:ring-2 focus-visible:ring-ring"
            >
              Applied here but not saved: {save.message}
            </span>
          </ScrollArea>
          <Button variant="outline" size="xs" onClick={onRetry}>
            Save again
          </Button>
        </>
      ) : (
        "Changes apply at once and are saved in this workspace."
      )}
    </div>
  )
}
