import * as React from "react"
import {
  Download01Icon,
  InformationCircleIcon,
} from "@hugeicons/core-free-icons"
import { useDebouncedCallback } from "@tanstack/react-pacer"
import { useSelector } from "@tanstack/react-store"

import { AboutSection } from "@/features/settings/about-section"
import { ConnectionsSettings } from "@/features/settings/connections-settings"
import {
  changePreferences,
  loadPreferences,
  preferencesStore,
  retrySavingPreferences,
} from "@/features/settings/preferences"
import { SettingsDialogView } from "@/features/settings/settings-dialog-view"
import {
  openSettings,
  closeSettings,
  useSettingsDialog,
} from "@/features/settings/settings-dialog-state"
import { UpdatesSection } from "@/features/updates/updates-section"
import type { SettingsSection } from "@/features/settings/settings-dialog-view"
import type { PreferencesChange } from "@/lib/ipc/settings"
import type { OpenConnection } from "@/lib/ipc/types"
import { useActionSource } from "@/lib/actions/context"

export type { SettingsSection } from "@/features/settings/settings-dialog-view"

/**
 * The settings dialog wired to the backend. Mount once, in the workspace
 * shell (and on the start screen if wanted); ⌘, opens it.
 */
export function SettingsDialog({
  sections = [],
  openConnections = [],
  onOpenConnectionChanged,
}: {
  sections?: Array<SettingsSection>
  /** Every connection with a workspace in the window, shown or hidden. */
  openConnections?: Array<OpenConnection>
  onOpenConnectionChanged?: (open: OpenConnection) => void
}) {
  const { isOpen, section, selectSection } = useSettingsDialog()
  const { preferences, save, loadError, palettesAvailable } =
    useSelector(preferencesStore)
  const [unsavedEdit, setUnsavedEdit] = React.useState(false)

  // `Settings…` of the registry: ⌘, and the menu bar, from every screen.
  useActionSource("settings", {}, { open: () => openSettings() })

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
      onSectionChange={selectSection}
      preferences={preferences}
      palettesAvailable={palettesAvailable}
      save={save}
      loadError={loadError}
      onReloadPreferences={() => void loadPreferences()}
      onPreferencesChange={onChange}
      onRetrySave={() => void retrySavingPreferences()}
      connections={
        <ConnectionsSettings
          openConnections={openConnections}
          onOpenConnectionChanged={onOpenConnectionChanged}
          onUnsavedEditChange={setUnsavedEdit}
        />
      }
      sections={[
        ...sections,
        // The computer's, not the workspace's: the section says so.
        {
          id: "updates",
          label: "Updates",
          icon: Download01Icon,
          content: <UpdatesSection />,
        },
        {
          id: "about",
          label: "About",
          icon: InformationCircleIcon,
          content: <AboutSection />,
        },
      ]}
      unsavedEdit={unsavedEdit}
    />
  )
}
