import * as React from "react"

import { results } from "@/lib/ipc/results"

/**
 * Releases, when the view closes, the reader that opening `result` counted in
 * the backend. The backend lets a result go at its last reader: a History tab
 * that never released would hold it forever, one that released for another
 * would take it from a view still reading it.
 */
export function useReleaseOnClose(result: string | null) {
  React.useEffect(() => {
    if (result === null) return
    return () => {
      void results.forgetResult(result).catch(() => undefined)
    }
  }, [result])
}
