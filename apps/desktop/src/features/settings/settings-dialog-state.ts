import { createStore, useSelector } from "@tanstack/react-store"

// Kept outside the view module: refreshing a settings component must not
// replace the store that callbacks in the workspace still address.
const settingsDialog = createStore({ open: false, section: "appearance" })

export function openSettings(section?: string) {
  settingsDialog.setState((state) => ({
    open: true,
    section: section ?? state.section,
  }))
}

export function closeSettings() {
  settingsDialog.setState((state) => ({ ...state, open: false }))
}

export function useSettingsDialog() {
  const state = useSelector(settingsDialog)
  return {
    isOpen: state.open,
    section: state.section,
    open: openSettings,
    close: closeSettings,
    selectSection: (section: string) =>
      settingsDialog.setState((current) => ({ ...current, section })),
  }
}
