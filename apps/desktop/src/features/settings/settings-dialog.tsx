import * as React from "react"
import { useHotkey } from "@tanstack/react-hotkeys"
import { useDebouncedCallback } from "@tanstack/react-pacer"
import { createStore, useStore } from "@tanstack/react-store"

import { ConnectionsSettings } from "@/features/settings/connections-settings"
import {
  changePreferences,
  loadPreferences,
  preferencesStore,
  retrySavingPreferences,
} from "@/features/settings/preferences"
import { SettingsDialogView } from "@/features/settings/settings-dialog-view"
import type { SettingsSection } from "@/features/settings/settings-dialog-view"
import type { PreferencesChange } from "@/lib/ipc/settings"
import type { OpenConnection } from "@/lib/ipc/types"

export type { SettingsSection } from "@/features/settings/settings-dialog-view"

const settingsDialog = createStore<{ open: boolean; section: string }>({
  open: false,
  section: "appearance",
})

/** Opens the settings, on a section if given. */
export function openSettings(section?: string) {
  settingsDialog.setState((state) => ({
    open: true,
    section: section ?? state.section,
  }))
}

export function closeSettings() {
  settingsDialog.setState((state) => ({ ...state, open: false }))
}

/** For a shell that needs `onOpenSettings`: `useSettingsDialog().open`. */
export function useSettingsDialog() {
  const state = useStore(settingsDialog)
  return {
    section: state.section,
    isOpen: state.open,
    open: openSettings,
    close: closeSettings,
  }
}

/**
 * The settings dialog wired to the backend. Mount once, in the workspace
 * shell (and on the start screen if wanted); ⌘, opens it.
 */
export function SettingsDialog({
  sections = [],
  openConnection = null,
  onOpenConnectionChanged,
}: {
  sections?: Array<SettingsSection>
  openConnection?: OpenConnection | null
  onOpenConnectionChanged?: (open: OpenConnection) => void
}) {
  const { isOpen, section } = useSettingsDialog()
  const { preferences, save, loadError } = useStore(preferencesStore)
  const [unsavedEdit, setUnsavedEdit] = React.useState(false)

  useHotkey("Mod+,", () => openSettings(), {
    preventDefault: true,
    ignoreInputs: false,
  })

  // The absent-value label is typed: one save when typing pauses, not one
  // revision per keystroke. Every other control saves on its gesture.
  const saveTyped = useDebouncedCallback(
    (change: PreferencesChange) => void changePreferences(change),
    { wait: 400 }
  )
  const onChange = (change: PreferencesChange) => {
    if (change.nullText !== undefined) {
      preferencesStore.setState((state) => ({
        ...state,
        preferences: { ...state.preferences, ...change },
      }))
      saveTyped(change)
    } else {
      void changePreferences(change)
    }
  }

  return (
    <SettingsDialogView
      open={isOpen}
      onOpenChange={(open) => (open ? openSettings() : closeSettings())}
      section={section}
      onSectionChange={(next) =>
        settingsDialog.setState((state) => ({ ...state, section: next }))
      }
      preferences={preferences}
      save={save}
      loadError={loadError}
      onReloadPreferences={() => void loadPreferences()}
      onPreferencesChange={onChange}
      onRetrySave={() => void retrySavingPreferences()}
      connections={
        <ConnectionsSettings
          openConnection={openConnection}
          onOpenConnectionChanged={onOpenConnectionChanged}
          onUnsavedEditChange={setUnsavedEdit}
        />
      }
      sections={sections}
      unsavedEdit={unsavedEdit}
    />
  )
}
