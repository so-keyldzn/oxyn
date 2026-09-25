import * as React from "react"
import { useStore } from "@tanstack/react-store"

import {
  changePreferencesFromView,
  preferencesStore,
} from "@/features/settings/preferences"
import type { DensityChoice } from "@/lib/ipc/settings"

/**
 * The text size as a result bar offers it: the reading preset of the
 * preferences, saved like any other change and failing like one (ADR-0013).
 * Nothing is rerun: `<html>` takes the preset, and the rows are redrawn.
 */
export function useResultDensity() {
  const value = useStore(
    preferencesStore,
    (state) => state.preferences.readingDensity
  )
  return React.useMemo(
    () => ({
      value,
      onChange: (density: DensityChoice) => {
        if (density !== value)
          void changePreferencesFromView({ readingDensity: density })
      },
    }),
    [value]
  )
}
