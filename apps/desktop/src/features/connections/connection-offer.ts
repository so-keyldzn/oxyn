import { createStore } from "@tanstack/react-store"

/**
 * A connection the user was offered by dropping a database file: the driver
 * and the one form field the file fills. An offer, not a connection — the
 * connection screen opens on it, in `production` like any new connection,
 * and nothing is saved until the user submits (UX-SPEC « Souris et glisser »).
 */
export interface ConnectionOffer {
  driver: string
  field: string
  path: string
  /** Bumped on each drop: the same file dropped again opens a fresh form. */
  id: number
}

export const connectionOffer = createStore<ConnectionOffer | null>(null)

let next = 0

export function offerConnection(offer: Omit<ConnectionOffer, "id">) {
  next += 1
  connectionOffer.setState(() => ({ ...offer, id: next }))
}

/** The screen took the offer: leaving and coming back does not replay it. */
export function clearConnectionOffer() {
  connectionOffer.setState(() => null)
}
