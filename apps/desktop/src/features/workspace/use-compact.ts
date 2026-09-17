import * as React from "react"

/** Below this width the side columns overlay the work area (docs/UX-SPEC.md). */
export const COMPACT_BELOW_PX = 1200

export function useCompact() {
  const query = `(max-width: ${COMPACT_BELOW_PX - 1}px)`
  return React.useSyncExternalStore(
    (onChange) => {
      const list = window.matchMedia(query)
      list.addEventListener("change", onChange)
      return () => list.removeEventListener("change", onChange)
    },
    () => window.matchMedia(query).matches,
    () => false
  )
}
