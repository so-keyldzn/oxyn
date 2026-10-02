import * as React from "react"
import { useStore } from "@tanstack/react-store"

import { UpdateSettings } from "@/components/oxyn/update-settings"
import { Spinner } from "@/components/ui/spinner"

import { restartNow } from "./restart"
import {
  cancelUpdate,
  checkForUpdates,
  downloadUpdate,
  openReleasePage,
  patchUpdates,
  retrySavingAutomatic,
  setAutomaticUpdates,
  updateStore,
} from "./update-store"

/** Settings ▸ Updates, wired to the backend (ADR-0051). */
export function UpdatesSection() {
  const view = useStore(updateStore)
  const heading = React.useRef<HTMLHeadingElement>(null)

  // `Check for updates…` lands here: the keyboard follows it.
  React.useEffect(() => {
    if (!view.focusRequested || view.snapshot === null) return
    heading.current?.focus()
    patchUpdates({ focusRequested: false })
  }, [view.focusRequested, view.snapshot])

  if (view.snapshot === null)
    return (
      <p className="flex items-center gap-2 text-sm text-muted-foreground">
        <Spinner /> Reading the update state…
      </p>
    )

  return (
    <UpdateSettings
      snapshot={view.snapshot}
      save={view.save}
      justUpdated={view.justUpdated}
      headingRef={heading}
      onCheck={checkForUpdates}
      onCancel={cancelUpdate}
      onDownload={downloadUpdate}
      onRestart={restartNow}
      onReleasePage={openReleasePage}
      onWhatsNew={openReleasePage}
      onAutomaticChange={(automatic) => void setAutomaticUpdates(automatic)}
      onRetrySave={retrySavingAutomatic}
    />
  )
}
