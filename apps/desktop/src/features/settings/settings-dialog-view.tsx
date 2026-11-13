import * as React from "react"
import { HugeiconsIcon } from "@hugeicons/react"
import {
  PaletteIcon,
  PlugSocketIcon,
  TextFontIcon,
} from "@hugeicons/core-free-icons"
import type { IconSvgElement } from "@hugeicons/react"

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
import { Kbd, KbdGroup } from "@/components/ui/kbd"
import { ScrollArea } from "@/components/ui/scroll-area"
import { Tabs, TabsContent, TabsList, TabsTrigger } from "@/components/ui/tabs"
import { actionKeys } from "@/lib/actions/manifest"
import { cn } from "@/lib/utils"
import type { DisplayPreferences, PreferencesChange } from "@/lib/ipc/settings"

// The generated trigger dims inactive tabs to foreground/60, which misses AA
// on the light popover (4.4:1). The muted token is the AA-checked one.
const TRIGGER =
  "h-9 flex-none justify-start px-3 text-muted-foreground max-sm:group-data-vertical/tabs:w-auto"

function SectionHeading({
  title,
  children,
}: {
  title: string
  children: React.ReactNode
}) {
  return (
    <header className="flex shrink-0 flex-col gap-1.5">
      <h2 className="text-base font-semibold">{title}</h2>
      <p className="text-sm text-muted-foreground">{children}</p>
    </header>
  )
}

/** A section another feature adds — the AI providers, for instance. */
export interface SettingsSection {
  id: string
  label: string
  /** Required: one section without an icon would shift its label out of line. */
  icon: IconSvgElement
  content: React.ReactNode
}

/** A tab's label, the section's icon ahead of it. */
function SectionLabel({
  icon,
  children,
}: {
  icon: IconSvgElement
  children: React.ReactNode
}) {
  return (
    <>
      <HugeiconsIcon icon={icon} strokeWidth={2} data-icon="inline-start" />
      {children}
    </>
  )
}

export interface SettingsDialogViewProps {
  open: boolean
  onOpenChange: (open: boolean) => void
  section: string
  onSectionChange: (section: string) => void
  preferences: DisplayPreferences
  palettesAvailable?: boolean
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
  palettesAvailable = true,
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
      <DialogContent className="flex h-[min(760px,calc(100dvh-2rem))] flex-col gap-0 overflow-hidden p-0 sm:max-w-4xl">
        <DialogHeader className="shrink-0 border-b px-6 py-5 pr-14">
          <DialogTitle className="text-lg">Settings</DialogTitle>
          <DialogDescription>
            Your workspace preferences and application settings. Open with{" "}
            <KbdGroup>
              {actionKeys("app.settings").map((key) => (
                <Kbd key={key}>{key}</Kbd>
              ))}
            </KbdGroup>
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
          className="min-h-0 flex-1 gap-0 max-sm:flex-col"
        >
          <div className="w-52 shrink-0 border-r p-3 max-sm:w-full max-sm:border-r-0 max-sm:border-b">
            <TabsList
              aria-label="Settings sections"
              className="w-full items-stretch gap-1 max-sm:flex-wrap max-sm:group-data-vertical/tabs:flex-row"
            >
              <TabsTrigger className={TRIGGER} value="appearance">
                <SectionLabel icon={PaletteIcon}>Appearance</SectionLabel>
              </TabsTrigger>
              <TabsTrigger className={TRIGGER} value="formats">
                <SectionLabel icon={TextFontIcon}>Formats</SectionLabel>
              </TabsTrigger>
              <TabsTrigger className={TRIGGER} value="connections">
                <SectionLabel icon={PlugSocketIcon}>Connections</SectionLabel>
              </TabsTrigger>
              {sections.map((extra) => (
                <TabsTrigger
                  key={extra.id}
                  className={TRIGGER}
                  value={extra.id}
                >
                  <SectionLabel icon={extra.icon}>{extra.label}</SectionLabel>
                </TabsTrigger>
              ))}
            </TabsList>
          </div>

          {/* `min-w-0`: a flex item is at least as wide as its content, and a
              long command line in a settings page would widen the whole
              dialog past its frame. */}
          <div className="flex min-h-0 min-w-0 flex-1 flex-col">
            <TabsContent
              value="connections"
              className="flex min-h-0 flex-1 flex-col gap-5 overflow-hidden p-6"
            >
              <SectionHeading title="Connections">
                Manage saved connections, their environments and privacy
                settings.
              </SectionHeading>
              {connections}
            </TabsContent>
            <ScrollArea
              className={cn(
                "min-h-0 min-w-0 flex-1",
                current === "connections" && "hidden"
              )}
            >
              <div className="flex min-w-0 flex-col gap-5 p-6">
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

                <TabsContent value="appearance" className="flex flex-col gap-6">
                  <SectionHeading title="Appearance">
                    Make the workspace comfortable to read. These choices do not
                    change your data.
                  </SectionHeading>
                  <AppearanceSettings
                    preferences={preferences}
                    palettesAvailable={palettesAvailable}
                    onChange={onPreferencesChange}
                  />
                </TabsContent>
                <TabsContent value="formats" className="flex flex-col gap-6">
                  <SectionHeading title="Value formats">
                    Choose how result values appear. Database values and
                    exported files stay unchanged.
                  </SectionHeading>
                  <FormatSettings
                    preferences={preferences}
                    onChange={onPreferencesChange}
                  />
                </TabsContent>
                {sections.map((extra) => (
                  <TabsContent key={extra.id} value={extra.id}>
                    {extra.content}
                  </TabsContent>
                ))}
              </div>
            </ScrollArea>
            {current === "appearance" || current === "formats" ? (
              <div className="shrink-0 border-t px-6 py-3">
                <PreferencesSaveStatus save={save} onRetry={onRetrySave} />
              </div>
            ) : null}
          </div>
        </Tabs>
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
