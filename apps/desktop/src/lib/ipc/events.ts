import { createStore } from "@tanstack/react-store"

import { backend } from "./client"
import type { ExecutionEvent, TransactionState } from "./types"

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

/**
 * The last transaction state each session reported, by session id
 * (ADR-0039). Changed by events only — never when a `BEGIN` or a `COMMIT` is
 * submitted: the engine may have rolled back on its own.
 */
export const transactionStates = createStore<Record<string, TransactionState>>(
  {}
)

/**
 * The session of each run that has not reported its state yet, by command.
 *
 * Apart from `executionProgress`, which forgets a command when its answer
 * arrives: the events travel on another channel, and may come after it. An
 * entry leaves when the run reports a state or ends; the rest — runs refused
 * before reaching the session — leave with their session.
 */
const awaitingState: Record<string, string> = {}

/** Records what a session reported at opening. */
export function recordTransactionState(
  session: string,
  state: TransactionState
) {
  transactionStates.setState((all) =>
    all[session] === state ? all : { ...all, [session]: state }
  )
}

/** Drops what is known about a closed session. */
export function forgetSession(session: string) {
  for (const [command, owner] of Object.entries(awaitingState))
    if (owner === session) delete awaitingState[command]
  transactionStates.setState((all) => {
    if (!(session in all)) return all
    const { [session]: _removed, ...rest } = all
    return rest
  })
}

/**
 * Starts tracking a run. `session` is the one it runs on, when the caller
 * knows it: a run abandoned by the backend ends on `cancelled` without
 * reporting a state, and its session then becomes `unknown`.
 */
export function watch(command: string, session: string | null = null) {
  executionProgress.setState((all) => ({ ...all, [command]: EMPTY }))
  if (session !== null) awaitingState[command] = session
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
  if (event.type === "transactionState") {
    delete awaitingState[event.command]
    recordTransactionState(event.session, event.state)
    return
  }
  if (
    event.type === "cancelled" ||
    event.type === "completed" ||
    event.type === "failed"
  ) {
    const session = awaitingState[event.command]
    delete awaitingState[event.command]
    // Events of one run arrive in order: a `cancelled` with no state before
    // it is an abandoned run, whose session nobody read (ADR-0039 §3).
    if (event.type === "cancelled" && session !== undefined)
      recordTransactionState(session, "unknown")
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
