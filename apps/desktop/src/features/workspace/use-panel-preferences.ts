import * as React from "react"
import { useStore } from "@tanstack/react-store"

import {
  changePreferencesFromView,
  preferencesStore,
} from "@/features/settings/preferences"

/**
 * The sidebar and the right column, as the workspace preferences keep them
 * (ADR-0013: `sidebar_collapsed`, `inspector_open`).
 *
 * At wide width the preference is the state: read from the snapshot, and each
 * toggle saved. Below 1200 px nothing is written — the panels keep their
 * wide-width preference, and widening the window gives it back (UX-SPEC,
 * « Largeur réduite »). The compact column starts closed and opens only on
 * demand; the compact sidebar is the layout's own.
 */
export function usePanelPreferences(compact: boolean) {
  const sidebarCollapsed = useStore(
    preferencesStore,
    (state) => state.preferences.sidebarCollapsed
  )
  const inspectorOpen = useStore(
    preferencesStore,
    (state) => state.preferences.inspectorOpen
  )
  // Tied to the width it was opened at: leaving compact drops it, and coming
  // back finds the column closed again.
  const [overlay, setOverlay] = React.useState({ compact, open: false })
  if (overlay.compact !== compact) setOverlay({ compact, open: false })
  const overlayOpen = overlay.compact === compact && overlay.open

  const setSidebarOpen = (open: boolean) => {
    if (compact || open === !sidebarCollapsed) return
    void changePreferencesFromView({ sidebarCollapsed: !open })
  }

  const setAsideOpen = (open: boolean) => {
    if (compact) {
      setOverlay({ compact, open })
      return
    }
    if (open === inspectorOpen) return
    void changePreferencesFromView({ inspectorOpen: open })
  }

  return {
    sidebarOpen: !sidebarCollapsed,
    setSidebarOpen,
    asideOpen: compact ? overlayOpen : inspectorOpen,
    setAsideOpen,
  }
}
