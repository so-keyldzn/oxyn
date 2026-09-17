import * as React from "react"
import { useStore } from "@tanstack/react-store"

import { loadPreferences, preferencesStore } from "./preferences"
import type { DensityChoice, ThemeChoice } from "@/lib/ipc/settings"

const DARK_QUERY = "(prefers-color-scheme: dark)"

/** Whether a theme choice resolves to the dark palette right now. */
export function resolvesToDark(theme: ThemeChoice, systemDark: boolean) {
  return theme === "system" ? systemDark : theme === "dark"
}

/**
 * Puts the palette and the reading density on `<html>`.
 *
 * On the root element and nowhere else: dialogs and menus are portalled
 * outside the application root and must pick up the same tokens. The density
 * is a data attribute that `styles.css` turns into row heights and text sizes.
 */
export function applyAppearance(
  root: HTMLElement,
  theme: ThemeChoice,
  density: DensityChoice,
  systemDark: boolean
) {
  const dark = resolvesToDark(theme, systemDark)
  root.classList.toggle("dark", dark)
  root.style.colorScheme = dark ? "dark" : "light"
  root.dataset.density = density
}

/** Reads the preferences at startup and keeps `<html>` in step with them. */
export function useAppearance() {
  const theme = useStore(preferencesStore, (state) => state.preferences.theme)
  const density = useStore(
    preferencesStore,
    (state) => state.preferences.readingDensity
  )

  React.useEffect(() => {
    void loadPreferences()
  }, [])

  React.useEffect(() => {
    const media = window.matchMedia(DARK_QUERY)
    const apply = () =>
      applyAppearance(document.documentElement, theme, density, media.matches)
    apply()
    // Only a `system` choice follows the operating system as it changes.
    if (theme !== "system") return
    media.addEventListener("change", apply)
    return () => media.removeEventListener("change", apply)
  }, [theme, density])
}
