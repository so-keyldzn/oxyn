import * as React from "react"
import { useStore } from "@tanstack/react-store"

import { NullTextContext } from "@/components/oxyn/cell-value"
import { preferencesStore } from "@/features/settings/preferences"

/**
 * Gives every cell drawn below it the absent-value marker of the preferences
 * (ADR-0013). A context and not the cached pages: a NULL crosses the IPC as
 * NULL, so a saved marker redraws the results already shown without a backend
 * read, and the next result takes it the same way.
 */
export function PreferredNullText({ children }: { children: React.ReactNode }) {
  const nullText = useStore(
    preferencesStore,
    (state) => state.preferences.nullText
  )
  return <NullTextContext value={nullText}>{children}</NullTextContext>
}
