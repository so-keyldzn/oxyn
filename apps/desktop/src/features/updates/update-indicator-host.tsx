import { useStore } from "@tanstack/react-store"

import { UpdateIndicator } from "@/components/oxyn/update-indicator"
import { pendingUpdate } from "@/components/oxyn/update-model"
import type { PendingUpdate } from "@/components/oxyn/update-model"
import { openSettings } from "@/features/settings/settings-dialog"
import { useCompact } from "@/features/workspace/use-compact"

import { restartNow } from "./restart"
import { openReleasePage, updateStore } from "./update-store"

/**
 * The indicator wired to the backend (ADR-0051): in a workspace's status bar,
 * or in the home screen's title bar. Nothing while no update is pending — a
 * background check that failed stays silent here.
 */
export function UpdateIndicatorHost() {
  const state = useStore(updateStore, (view) => view.snapshot?.state ?? null)
  const pending = state ? pendingUpdate(state) : null
  if (pending === null) return null
  return <PendingUpdateIndicator pending={pending} />
}

/** The window width is watched only while something is pending. */
function PendingUpdateIndicator({ pending }: { pending: PendingUpdate }) {
  const compact = useCompact()
  return (
    <UpdateIndicator
      pending={pending}
      compact={compact}
      onRestart={restartNow}
      onWhatsNew={openReleasePage}
      onOpenSettings={() => openSettings("updates")}
      onReleasePage={openReleasePage}
    />
  )
}
