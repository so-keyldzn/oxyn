// What the catalog and the assistant of one connection share, outside React.
//
// The catalog sidebar offers « Pin to question » and the assistant panel sends
// it: they are siblings in the workspace, and neither can hand props to the
// other. The chosen destination lives here for the same reason — the menu
// shows the action only for a built-in provider, and must read the choice the
// panel made, not guess it again.

import * as React from "react"

import { selectedDestination } from "./availability"
import { useAssistantAvailable } from "./use-assistant-available"
import { addressKey, addressLabel } from "@/components/oxyn/catalog-tree"
import type { PinToQuestion } from "@/components/oxyn/catalog-tree"
import type {
  CatalogAddress,
  CatalogNode,
  OpenConnection,
} from "@/lib/ipc/types"

/**
 * One object whose rows may go with the next question, once approved.
 *
 * Only names: no column, no value. What is offered is read again by the
 * backend when the sample is asked for.
 */
export interface ObjectPin {
  key: string
  label: string
  address: CatalogAddress
}

export interface PinState {
  /** The destination picked in the panel; `null` keeps the default one. */
  chosenKey: string | null
  /**
   * At most one: a question carries one approved sample, and a second pin
   * would suggest a second sample the backend never offers.
   */
  pin: ObjectPin | null
}

const states = new Map<string, PinState>()
const listeners = new Map<string, Set<() => void>>()

// Who answers is remembered per connection across restarts, in the webview's
// own storage. Not in the workspace preferences, which the backend owns and
// which hold nothing per connection: a key there is a change to the domain.
// What is kept is only `provider:<id>` or `agent:<id>` — two opaque ids, no
// label, no secret — and it is a convenience: a value that no longer names a
// declared destination falls back to the default (`selectedDestination`), and
// storage that refuses is not an error.
const DESTINATION_KEY = "oxyn.assistant.destination."

function recalledDestination(connection: string): string | null {
  try {
    return localStorage.getItem(DESTINATION_KEY + connection)
  } catch {
    return null
  }
}

function rememberDestination(connection: string, key: string) {
  try {
    localStorage.setItem(DESTINATION_KEY + connection, key)
  } catch {
    /* storage unavailable: the choice holds for this window */
  }
}

function publish(connection: string, update: (state: PinState) => PinState) {
  states.set(connection, update(getPinState(connection)))
  for (const notify of listeners.get(connection) ?? []) notify()
}

export function getPinState(connection: string): PinState {
  let found = states.get(connection)
  if (!found) {
    // Read once per connection and window, then kept: `useSyncExternalStore`
    // needs the same object back on every read.
    found = { chosenKey: recalledDestination(connection), pin: null }
    states.set(connection, found)
  }
  return found
}

export function usePinState(connection: string): PinState {
  return React.useSyncExternalStore(
    React.useCallback(
      (notify: () => void) => {
        const found = listeners.get(connection) ?? new Set()
        listeners.set(connection, found)
        found.add(notify)
        return () => {
          found.delete(notify)
        }
      },
      [connection]
    ),
    () => getPinState(connection),
    () => getPinState(connection)
  )
}

export function chooseDestination(connection: string, key: string) {
  rememberDestination(connection, key)
  publish(connection, (state) => ({ ...state, chosenKey: key }))
}

/** Replaces the pin: one object per question. */
export function pinObject(connection: string, node: CatalogNode) {
  publish(connection, (state) => ({
    ...state,
    pin: {
      key: addressKey(node.address),
      label: addressLabel(node.address),
      address: node.address,
    },
  }))
}

export function unpin(connection: string) {
  publish(connection, (state) =>
    state.pin === null ? state : { ...state, pin: null }
  )
}

/**
 * « Pin to question » for the catalog of `open`, decided from the connection's
 * tier and the destination the assistant would use. `canPin` applies it.
 */
export function usePinToQuestion(open: OpenConnection): PinToQuestion {
  const entry = useAssistantAvailable(open)
  const { chosenKey } = usePinState(open.connection)
  const selected =
    entry.status === "enabled"
      ? selectedDestination(entry.destinations, chosenKey)
      : null
  return {
    tier: open.privacyTier,
    destination: selected?.usable ? selected.kind : null,
    onPin: (node) => pinObject(open.connection, node),
  }
}
