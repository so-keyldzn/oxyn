import * as React from "react"

import {
  AppearanceSettings,
  PreferencesSaveStatus,
} from "@/components/oxyn/appearance-settings"
import type { SaveStatus } from "@/components/oxyn/appearance-settings"
import { BackendErrorAlert } from "@/components/oxyn/backend-error-alert"
import { DiscardChangesDialog } from "@/components/oxyn/discard-changes-dialog"
import type { BackendFailure } from "@/components/oxyn/backend-error-alert"
import { FormatSettings } from "@/components/oxyn/format-settings"
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog"
import { Kbd } from "@/components/ui/kbd"
import { ScrollArea } from "@/components/ui/scroll-area"
import { Tabs, TabsContent, TabsList, TabsTrigger } from "@/components/ui/tabs"
import type { DisplayPreferences, PreferencesChange } from "@/lib/ipc/settings"

// The generated trigger dims inactive tabs to foreground/60, which misses AA
// on the light popover (4.4:1). The muted token is the AA-checked one.
const TRIGGER = "text-muted-foreground"

/** A section another feature adds — the AI providers, for instance. */
export interface SettingsSection {
  id: string
  label: string
  content: React.ReactNode
}

export interface SettingsDialogViewProps {
  open: boolean
  onOpenChange: (open: boolean) => void
  section: string
  onSectionChange: (section: string) => void
  preferences: DisplayPreferences
  save: SaveStatus
  loadError?: BackendFailure | null
  onReloadPreferences: () => void
  onPreferencesChange: (change: PreferencesChange) => void
  onRetrySave: () => void
  /** The connections section, wired by the container. */
  connections: React.ReactNode
  sections?: Array<SettingsSection>
  /**
   * A connection edit holds typed values: closing the dialog or changing
   * section asks before they are lost.
   */
  unsavedEdit?: boolean
}

/**
 * The settings: appearance, cell formats, saved connections, and the sections
 * other features inject. Vertical tabs, one scrolling panel; the save state of
 * preferences stays at the bottom of the panels it concerns.
 *
 * While `unsavedEdit`, Esc, Close and another section open a confirmation
 * whose default keeps the edit; preferences save on each gesture and never
 * need one.
 */
export function SettingsDialogView({
  open,
  onOpenChange,
  section,
  onSectionChange,
  preferences,
  save,
  loadError,
  onReloadPreferences,
  onPreferencesChange,
  onRetrySave,
  connections,
  sections = [],
  unsavedEdit = false,
}: SettingsDialogViewProps) {
  const [pendingExit, setPendingExit] = React.useState<
    { kind: "close" } | { kind: "section"; section: string } | null
  >(null)

  const known = [
    "appearance",
    "formats",
    "connections",
    ...sections.map((s) => s.id),
  ]
  const current = known.includes(section) ? section : "appearance"

  return (
    <Dialog
      open={open}
      onOpenChange={(next) => {
        if (!next && unsavedEdit) setPendingExit({ kind: "close" })
        else onOpenChange(next)
      }}
    >
      <DialogContent className="flex h-[min(680px,calc(100vh-2rem))] flex-col gap-3 sm:max-w-3xl">
        <DialogHeader>
          <DialogTitle>Settings</DialogTitle>
          <DialogDescription>
            Saved in this workspace. Open them from anywhere with <Kbd>⌘</Kbd>
            <Kbd>,</Kbd>
          </DialogDescription>
        </DialogHeader>

        <Tabs
          orientation="vertical"
          value={current}
          onValueChange={(value: unknown) => {
            if (typeof value !== "string" || value === current) return
            if (unsavedEdit) setPendingExit({ kind: "section", section: value })
            else onSectionChange(value)
          }}
          className="min-h-0 flex-1 gap-4 max-sm:flex-col"
        >
          <TabsList
            variant="line"
            className="w-44 shrink-0 items-stretch max-sm:w-full max-sm:flex-row max-sm:overflow-x-auto"
          >
            <TabsTrigger className={TRIGGER} value="appearance">
              Appearance
            </TabsTrigger>
            <TabsTrigger className={TRIGGER} value="formats">
              Formats
            </TabsTrigger>
            <TabsTrigger className={TRIGGER} value="connections">
              Connections
            </TabsTrigger>
            {sections.map((extra) => (
              <TabsTrigger key={extra.id} className={TRIGGER} value={extra.id}>
                {extra.label}
              </TabsTrigger>
            ))}
          </TabsList>

          <ScrollArea className="min-h-0 flex-1">
            <div className="flex flex-col gap-4 pr-3 pb-1">
              {loadError &&
              (current === "appearance" || current === "formats") ? (
                <BackendErrorAlert
                  title="Preferences could not be read"
                  error={loadError}
                  onRetry={onReloadPreferences}
                  retryLabel="Read again"
                  nextStep="The defaults are shown; nothing is saved over the stored state."
                />
              ) : null}

              <TabsContent value="appearance">
                <AppearanceSettings
                  preferences={preferences}
                  onChange={onPreferencesChange}
                />
              </TabsContent>
              <TabsContent value="formats">
                <FormatSettings
                  preferences={preferences}
                  onChange={onPreferencesChange}
                />
              </TabsContent>
              <TabsContent value="connections">{connections}</TabsContent>
              {sections.map((extra) => (
                <TabsContent key={extra.id} value={extra.id}>
                  {extra.content}
                </TabsContent>
              ))}
            </div>
          </ScrollArea>
        </Tabs>

        {current === "appearance" || current === "formats" ? (
          <PreferencesSaveStatus save={save} onRetry={onRetrySave} />
        ) : null}
      </DialogContent>
      <DiscardChangesDialog
        open={pendingExit !== null}
        what="the changes to this connection"
        onKeep={() => setPendingExit(null)}
        onDiscard={() => {
          const exit = pendingExit
          setPendingExit(null)
          if (exit?.kind === "close") onOpenChange(false)
          else if (exit) onSectionChange(exit.section)
        }}
      />
    </Dialog>
  )
}
