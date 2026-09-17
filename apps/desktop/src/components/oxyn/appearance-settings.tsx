import { HugeiconsIcon } from "@hugeicons/react"
import { Alert02Icon, CheckmarkCircle02Icon } from "@hugeicons/core-free-icons"

import { Button } from "@/components/ui/button"
import {
  FieldDescription,
  FieldGroup,
  FieldLegend,
  FieldSet,
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

const DENSITIES: ReadonlyArray<{
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
 * width, and neither reruns a query (docs/UX-SPEC.md « Lisibilité et hauteur
 * de grille »).
 */
export function AppearanceSettings({
  preferences,
  onChange,
}: {
  preferences: DisplayPreferences
  onChange: (change: PreferencesChange) => void
}) {
  return (
    <FieldGroup>
      <FieldSet>
        <FieldLegend variant="label">Theme</FieldLegend>
        <FieldDescription>
          « System » follows the operating system as it changes.
        </FieldDescription>
        <ToggleGroup
          variant="outline"
          spacing={0}
          value={[preferences.theme]}
          onValueChange={(next: Array<unknown>) => {
            const chosen = THEMES.find((theme) => theme.value === next[0])
            if (chosen) onChange({ theme: chosen.value })
          }}
          aria-label="Theme"
        >
          {THEMES.map((theme) => (
            <ToggleGroupItem key={theme.value} value={theme.value}>
              {theme.label}
            </ToggleGroupItem>
          ))}
        </ToggleGroup>
      </FieldSet>

      <FieldSet>
        <FieldLegend variant="label">Reading comfort</FieldLegend>
        <FieldDescription>Applies to text and result rows.</FieldDescription>
        <ToggleGroup
          variant="outline"
          spacing={0}
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
      className="flex min-h-7 items-center gap-2 text-xs text-muted-foreground"
    >
      {save.status === "saving" ? (
        <>
          <Spinner className="size-3" />
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
          <span data-selectable className="text-foreground">
            Applied here but not saved: {save.message}
          </span>
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
