import { afterEach, describe, expect, it, vi } from "vitest"

import {
  DEFAULT_PREFERENCES,
  preferencesStore,
  reloadPreferences,
} from "./preferences"

const written = vi.hoisted(() => ({
  preferences: null as null | Record<string, unknown>,
}))

vi.mock("@/lib/ipc/settings", () => ({
  settingsBackend: {
    readPreferences: () =>
      Promise.resolve({ preferences: written.preferences, revision: 2 }),
  },
}))

describe("preferences written by another window (ADR-0043)", () => {
  afterEach(() => {
    preferencesStore.setState((current) => ({
      ...current,
      preferences: DEFAULT_PREFERENCES,
      formatRevision: 0,
    }))
  })

  it("follows the display at once and keeps this window's layout", async () => {
    written.preferences = {
      ...DEFAULT_PREFERENCES,
      theme: "light",
      sidebarCollapsed: true,
      inspectorOpen: false,
      inspectorWidth: 400,
    }
    await reloadPreferences()
    const { preferences, formatRevision } = preferencesStore.state
    expect(preferences.theme).toBe("light")
    expect(preferences.sidebarCollapsed).toBe(false)
    expect(preferences.inspectorOpen).toBe(true)
    expect(preferences.inspectorWidth).toBe(280)
    expect(formatRevision).toBe(0)
  })

  it("says the cached pages are stale when the cell format changed", async () => {
    written.preferences = { ...DEFAULT_PREFERENCES, nullText: "NULL" }
    await reloadPreferences()
    expect(preferencesStore.state.preferences.nullText).toBe("NULL")
    expect(preferencesStore.state.formatRevision).toBe(1)
  })
})
