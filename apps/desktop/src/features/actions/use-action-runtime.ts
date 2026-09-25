import * as React from "react"

import {
  actionSources,
  currentContext,
  focusStore,
  trackFocus,
} from "@/lib/actions/context"
import { installKeyboard } from "@/lib/actions/keyboard"
import { manifest } from "@/lib/actions/manifest"
import { entryStates } from "@/lib/actions/menu-model"
import { nativeMenu } from "@/lib/actions/platform"
import { invoke } from "@/lib/actions/registry"
import { menu } from "@/lib/ipc/menu"
import type { MenuEntryState } from "@/lib/ipc/menu"

/** The states of the native bar's entries, as `set_menu_state` takes them. */
function nativeStates(): Array<MenuEntryState> {
  return entryStates(manifest, "mac", currentContext()).map((entry) => ({
    id: entry.id,
    enabled: entry.availability === true,
    variant: entry.variant,
    shortcut: entry.shortcut,
    checked: entry.checked,
  }))
}

/**
 * Starts the registry's triggers for the whole window, once: the keyboard
 * dispatcher, the focus zones, and on macOS the native bar in both
 * directions (docs/adr/0041-registre-d-actions-menus-et-raccourcis.md).
 */
export function useActionRuntime() {
  React.useEffect(() => {
    const stopFocus = trackFocus()
    const stopKeyboard = installKeyboard()
    if (!nativeMenu) {
      return () => {
        stopKeyboard()
        stopFocus()
      }
    }
    // Sent only when it changed: the context moves on focus, tab, selection
    // or execution state, and most of those leave every entry as it was.
    let sent = ""
    const push = () => {
      const states = nativeStates()
      const key = JSON.stringify(states)
      if (key === sent) return
      sent = key
      void menu.setState(states).catch((error: unknown) => {
        sent = ""
        console.error(`The menu bar refused its state: ${String(error)}`)
      })
    }
    void menu
      .subscribe((activation) => {
        invoke(activation.id, "menu")
        // AppKit flips a check entry's mark on click: choosing the theme
        // already chosen changes no state here, and the mark must come back.
        sent = ""
        push()
      })
      .catch((error: unknown) =>
        console.error(`The menu bar cannot reach this window: ${String(error)}`)
      )
    push()
    const sources = actionSources.subscribe(push)
    const focus = focusStore.subscribe(push)
    return () => {
      sources.unsubscribe()
      focus.unsubscribe()
      stopKeyboard()
      stopFocus()
    }
  }, [])
}
