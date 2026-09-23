import { createStore } from "@tanstack/react-store"

import type { CatalogAddress } from "@/lib/ipc/types"

/** An object of the catalog to open in a workspace's tab. Nothing runs. */
export interface ObjectOpenRequest {
  id: number
  /** The workspace it belongs to: another connection's screen ignores it. */
  connection: string
  address: CatalogAddress
}

// A queue rather than a callback prop, like `openInConsole`: the assistant is
// mounted in the right column by `useAsidePanels`, beside the workspace screen
// that owns the tabs, and neither can hand props to the other.
export const objectOpenRequests = createStore<Array<ObjectOpenRequest>>([])

let next = 0

/**
 * Opens `address` in the workspace of `connection`, as a click in the catalog
 * would: its preview is an ordinary read on the bus, as the user.
 */
export function openObject(connection: string, address: CatalogAddress) {
  next += 1
  objectOpenRequests.setState((requests) => [
    ...requests,
    { id: next, connection, address },
  ])
}

/** The requests for `connection`, taken out of the queue; the rest stay. */
export function takeObjectOpenRequests(connection: string) {
  const mine = objectOpenRequests.state.filter(
    (request) => request.connection === connection
  )
  if (mine.length > 0)
    objectOpenRequests.setState((requests) =>
      requests.filter((request) => request.connection !== connection)
    )
  return mine
}
