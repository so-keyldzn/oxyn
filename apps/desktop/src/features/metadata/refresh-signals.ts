import * as React from "react"

import { metadata } from "@/lib/ipc/metadata"
import type { RefreshSignal } from "@/lib/ipc/metadata"

type Listener = (signal: RefreshSignal) => void

const listeners = new Set<Listener>()
let subscribed = false

function subscribe() {
  if (subscribed) return
  subscribed = true
  metadata
    .subscribeRefreshSignals((signal) => {
      for (const listener of listeners) listener(signal)
    })
    .catch((error: unknown) => {
      subscribed = false
      console.error("refresh signals unavailable", error)
    })
}

/**
 * Calls `onSignal` when something shown for `connection` should be read again
 * (ADR-0022): a DDL invalidated the catalog, a write changed rows, or the
 * channel lagged and nobody knows what changed.
 *
 * Signals for another connection are ignored. The caller decides what is on
 * screen and reads only that: a hidden tab reads again when it comes back.
 */
export function useRefreshSignal(
  connection: string,
  onSignal: (signal: RefreshSignal) => void
) {
  const latest = React.useRef(onSignal)
  latest.current = onSignal
  React.useEffect(() => {
    subscribe()
    const listener: Listener = (signal) => {
      if (signal.type !== "lagged" && signal.connection !== connection) return
      latest.current(signal)
    }
    listeners.add(listener)
    return () => {
      listeners.delete(listener)
    }
  }, [connection])
}
