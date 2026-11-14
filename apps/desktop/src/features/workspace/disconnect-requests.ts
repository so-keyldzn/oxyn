import { createStore } from "@tanstack/react-store"

/** Disconnect requests reveal their owner before any draft/transaction review. */
export const disconnectRequests = createStore<ReadonlyArray<string>>([])
const claimed = new Set<string>()
const completions = new Map<string, Array<(closed: boolean) => void>>()
export interface DisconnectCost {
  consoles: number
  unsaved: number
  running: number
}
const costs = new Map<string, () => DisconnectCost>()
export function registerDisconnectCosts(
  connection: string,
  read: () => DisconnectCost
) {
  costs.set(connection, read)
  return () => {
    if (costs.get(connection) === read) costs.delete(connection)
  }
}
export function readDisconnectCosts(connection: string): DisconnectCost | null {
  return costs.get(connection)?.() ?? null
}
export function requestWorkspaceDisconnect(connection: string) {
  disconnectRequests.setState((all) =>
    all.includes(connection) ? all : [...all, connection]
  )
}
/** Batch disconnect waits for the actual owner's draft review before proceeding. */
export function reviewWorkspaceDisconnect(
  connection: string
): Promise<boolean> {
  return new Promise((resolve) => {
    completions.set(connection, [
      ...(completions.get(connection) ?? []),
      resolve,
    ])
    requestWorkspaceDisconnect(connection)
  })
}
export function takeWorkspaceDisconnect(connection: string) {
  if (disconnectRequests.state[0] !== connection || claimed.has(connection))
    return false
  claimed.add(connection)
  return true
}
export function finishWorkspaceDisconnect(connection: string, closed: boolean) {
  claimed.delete(connection)
  disconnectRequests.setState((all) => all.filter((id) => id !== connection))
  const pending = completions.get(connection) ?? []
  completions.delete(connection)
  pending.forEach((resolve) => resolve(closed))
}
