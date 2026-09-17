import { createStore } from "@tanstack/react-store"

import { backend } from "./client"
import type { ExecutionEvent } from "./types"

/** What the front knows about a command while it runs. */
export interface CommandProgress {
  /** The result id, known from the schema onward: the grid can page it early. */
  result: string | null
  rows: number
  failed: { error: string; retryable: boolean } | null
  done: boolean
}

const EMPTY: CommandProgress = {
  result: null,
  rows: 0,
  failed: null,
  done: false,
}

/**
 * Progress by command id, fed by the backend's event channel.
 *
 * Only commands someone is watching are kept: an entry is created by `watch`
 * and removed by `forget`, so a long session does not accumulate every
 * statement it ever ran.
 */
export const executionProgress = createStore<Record<string, CommandProgress>>(
  {}
)

/** A monotonic counter bumped when the catalog changed on the backend. */
export const catalogVersion = createStore(0)

export function watch(command: string) {
  executionProgress.setState((all) => ({ ...all, [command]: EMPTY }))
}

export function forget(command: string) {
  executionProgress.setState((all) => {
    const { [command]: _removed, ...rest } = all
    return rest
  })
}

export function applyEvent(event: ExecutionEvent) {
  if (event.type === "catalogUpdated") {
    catalogVersion.setState((version) => version + 1)
    return
  }
  executionProgress.setState((all) => {
    const current = all[event.command]
    if (!current) return all
    switch (event.type) {
      case "schemaReady":
        return { ...all, [event.command]: { ...current, result: event.result } }
      case "batchReady":
        return {
          ...all,
          [event.command]: {
            ...current,
            result: event.result,
            rows: current.rows + event.rows,
          },
        }
      case "progress":
        return { ...all, [event.command]: { ...current, rows: event.rows } }
      case "completed":
        return {
          ...all,
          [event.command]: {
            ...current,
            result: event.result,
            rows: event.rows,
            done: true,
          },
        }
      case "failed":
        return {
          ...all,
          [event.command]: {
            ...current,
            failed: { error: event.error, retryable: event.retryable },
            done: true,
          },
        }
      case "cancelled":
        return { ...all, [event.command]: { ...current, done: true } }
      case "approvalRequested":
        return all
    }
  })
}

let subscribed = false

/** Opens the event channel once per window load. */
export function subscribeToBackendEvents() {
  if (subscribed) return
  subscribed = true
  backend.subscribeEvents(applyEvent).catch((error: unknown) => {
    subscribed = false
    console.error("execution events unavailable", error)
  })
}
