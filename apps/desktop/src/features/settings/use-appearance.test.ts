import { act, cleanup, renderHook, waitFor } from "@testing-library/react"
import { afterEach, describe, expect, it, vi } from "vitest"

import { applyAppearance, useAppearance } from "./use-appearance"
import { DEFAULT_PREFERENCES, preferencesStore } from "./preferences"
import { PreferencesState } from "@/lib/ipc/settings"
import type * as SettingsIpc from "@/lib/ipc/settings"

vi.mock("@/lib/ipc/settings", async (importOriginal) => {
  const actual = await importOriginal<typeof SettingsIpc>()
  return {
    ...actual,
    settingsBackend: {
      ...actual.settingsBackend,
      readPreferences: async () => ({
        revision: 1,
        palettesAvailable: true,
        preferences: {
          ...DEFAULT_PREFERENCES,
          theme: "system",
          lightPalette: "ocean",
          darkPalette: "plum",
        },
      }),
    },
  }
})

afterEach(() => {
  cleanup()
  vi.unstubAllGlobals()
})

describe("independent palettes and appearance mode", () => {
  it("resolves System without replacing either saved palette", () => {
    const root = document.createElement("div")
    const palettes = { lightPalette: "ocean", darkPalette: "plum" } as const
    applyAppearance(root, "system", "compact", false, palettes)
    expect(root.dataset.palette).toBe("ocean")
    expect(root.classList.contains("dark")).toBe(false)
    applyAppearance(root, "system", "compact", true, palettes)
    expect(root.dataset.palette).toBe("plum")
    expect(root.classList.contains("dark")).toBe(true)
    applyAppearance(root, "light", "compact", true, palettes)
    expect(root.dataset.palette).toBe("ocean")
    applyAppearance(root, "dark", "compact", false, palettes)
    expect(root.dataset.palette).toBe("plum")
    expect(palettes).toEqual({ lightPalette: "ocean", darkPalette: "plum" })
  })

  it("follows an OS change through the live subscription and keeps the preferences", async () => {
    const listeners = new Set<() => void>()
    const media = {
      matches: false,
      addEventListener: (_event: string, listener: () => void) =>
        listeners.add(listener),
      removeEventListener: (_event: string, listener: () => void) =>
        listeners.delete(listener),
    }
    vi.stubGlobal("matchMedia", () => media)
    const { unmount } = renderHook(() => useAppearance())
    await waitFor(() =>
      expect(document.documentElement.dataset.palette).toBe("ocean")
    )
    act(() => {
      media.matches = true
      listeners.forEach((listener) => listener())
    })
    expect(document.documentElement.dataset.palette).toBe("plum")
    expect(preferencesStore.state.preferences.lightPalette).toBe("ocean")
    expect(preferencesStore.state.preferences.darkPalette).toBe("plum")
    unmount()
    expect(listeners.size).toBe(0)
  })

  it("keeps an old host readable without offering unsupported palette writes", () => {
    const {
      lightPalette: _light,
      darkPalette: _dark,
      ...legacy
    } = DEFAULT_PREFERENCES
    const old = PreferencesState.parse({ revision: 1, preferences: legacy })
    expect(old.palettesAvailable).toBe(false)
    expect(old.preferences.lightPalette).toBe("oxyn")
    const current = PreferencesState.parse({
      revision: 2,
      preferences: DEFAULT_PREFERENCES,
    })
    expect(current.palettesAvailable).toBe(true)
    expect(
      PreferencesState.safeParse({
        revision: 2,
        preferences: { ...legacy, lightPalette: "ocean" },
      }).success
    ).toBe(false)
    expect(
      PreferencesState.safeParse({
        revision: 2,
        preferences: { ...DEFAULT_PREFERENCES, darkPalette: "unknown" },
      }).success
    ).toBe(false)
  })
})
