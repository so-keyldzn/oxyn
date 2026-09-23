// The external agent started before its first question, per connection,
// outside React.
//
// An agent declares its models, efforts and options only once its session is
// open. The panel starts it as soon as it shows the agent, so those selectors
// are there before anything is asked; the backend keeps the agent for the
// first question, which takes it rather than launching another.
//
// Outside React for the same reason as the conversations: closing the panel
// must not lose an answer that is on its way, and two panels of one
// connection must not start two agents. Never retried: after a failure, a new
// start is the user's click (I-13).

import * as React from "react"

import type { AgentSettings } from "./transcript"
import type { SignInState } from "./conversation-store"
import { ai } from "@/lib/ipc/ai"
import type {
  AgentExit,
  AgentStartRequest,
  FailureCategory,
  SignInHelp,
} from "@/lib/ipc/ai"

/** Why a start failed, as the backend classified it. */
export interface AgentStartFailure {
  message: string
  category: FailureCategory
  signIn: SignInHelp | null
  foundElsewhere: string | null
  exit: AgentExit | null
}

export type AgentStartup =
  | { status: "idle" }
  /** `shown` turns on after a moment: a start answered at once never flashes. */
  | { status: "starting"; key: string; label: string; shown: boolean }
  /** Stopped by the user before it was ready; the first question starts it. */
  | { status: "stopped"; key: string; label: string }
  | {
      status: "ready"
      key: string
      label: string
      version: string | null
      settings: AgentSettings
    }
  | { status: "failed"; key: string; label: string; failure: AgentStartFailure }

interface Slot {
  state: AgentStartup
  /** Answers of an older start are dropped. */
  generation: number
  request: AgentStartRequest | null
  signIn: Record<string, SignInState>
  listeners: Set<() => void>
}

const IDLE: AgentStartup = { status: "idle" }

/** How long a start may answer before « Starting » is drawn. */
export const STARTING_SHOWN_AFTER_MS = 400

const slots = new Map<string, Slot>()

function slot(connection: string): Slot {
  let found = slots.get(connection)
  if (!found) {
    found = {
      state: IDLE,
      generation: 0,
      request: null,
      signIn: {},
      listeners: new Set(),
    }
    slots.set(connection, found)
  }
  return found
}

function publish(connection: string, change: (found: Slot) => void) {
  const found = slot(connection)
  change(found)
  found.listeners.forEach((notify) => notify())
}

function message(error: unknown) {
  return error instanceof Error ? error.message : String(error)
}

/**
 * What identifies a start: the agent, the session its tools run in, and the
 * question it would answer. The same key is never started twice on its own —
 * not after a failure, not after a stop.
 */
export function startKey(request: AgentStartRequest) {
  return [
    request.agent,
    request.session,
    request.thread ?? "",
    request.parent ?? "",
  ].join("|")
}

export function getAgentStartup(connection: string): AgentStartup {
  return slot(connection).state
}

export function getAgentSignIn(connection: string) {
  return slot(connection).signIn
}

/**
 * Starts the agent unless this very start already ran — whatever its end.
 * `force` is the user's click: « Start again », or a sign-in just done.
 */
export async function startAgent(
  connection: string,
  request: AgentStartRequest,
  label: string,
  force = false
) {
  const key = startKey(request)
  const current = slot(connection).state
  if (!force && current.status !== "idle" && current.key === key) return
  let generation = 0
  publish(connection, (found) => {
    found.generation += 1
    generation = found.generation
    found.request = request
    found.signIn = force ? found.signIn : {}
    found.state = { status: "starting", key, label, shown: false }
  })
  const reveal = setTimeout(() => {
    const found = slot(connection)
    if (found.generation !== generation || found.state.status !== "starting")
      return
    const starting = found.state
    publish(connection, (same) => {
      same.state = { ...starting, shown: true }
    })
  }, STARTING_SHOWN_AFTER_MS)
  const settle = (state: AgentStartup) => {
    clearTimeout(reveal)
    if (slot(connection).generation !== generation) return
    publish(connection, (found) => {
      found.state = state
    })
  }
  try {
    const started = await ai.startAgent(request)
    switch (started.state) {
      case "ready":
        settle({
          status: "ready",
          key,
          label,
          version: started.version,
          settings: started.settings,
        })
        return
      case "cancelled":
        settle({ status: "stopped", key, label })
        return
      case "failed":
        settle({
          status: "failed",
          key,
          label,
          failure: {
            message: started.message,
            category: started.category,
            signIn: started.signIn,
            foundElsewhere: started.foundElsewhere,
            exit: started.exit,
          },
        })
        return
    }
  } catch (error) {
    // A refusal before anything started — the tier, a declaration gone: said
    // in the backend's words, as a failure nobody retries.
    settle({
      status: "failed",
      key,
      label,
      failure: {
        message: message(error),
        category: "refused",
        signIn: null,
        foundElsewhere: null,
        exit: null,
      },
    })
  }
}

/** The last start's own request, again: the user asked for it. */
export function startAgentAgain(connection: string) {
  const found = slot(connection)
  const state = found.state
  if (found.request === null || state.status === "idle") return
  void startAgent(connection, found.request, state.label, true)
}

/**
 * Stops the agent started ahead of a question — starting or ready — and
 * forgets it here. `keepStopped` leaves « stopped » drawn, for a click on
 * Cancel; otherwise the panel moved to someone else and nothing is drawn.
 */
export async function stopAgentStart(connection: string, keepStopped = false) {
  const current = slot(connection).state
  if (current.status === "idle") return
  publish(connection, (found) => {
    found.generation += 1
    found.state =
      keepStopped && current.status !== "stopped"
        ? { status: "stopped", key: current.key, label: current.label }
        : keepStopped
          ? current
          : IDLE
    if (!keepStopped) found.request = null
  })
  try {
    await ai.stopAgentStart(connection)
  } catch {
    // Nothing to tell: the agent ends with the connection all the same.
  }
}

/** The settings the started agent answered with, after a change. */
export function updateStartedSettings(
  connection: string,
  settings: AgentSettings
) {
  const current = slot(connection).state
  if (current.status !== "ready") return
  publish(connection, (found) => {
    found.state = { ...current, settings }
  })
}

/**
 * Signs in with the started agent, then starts it again: a session refused
 * before sign-in opens only once signed in. The agent does it; Oxyn sees no
 * token.
 */
export async function signInStartedAgent(connection: string, method: string) {
  const mark = (value: SignInState) =>
    publish(connection, (found) => {
      found.signIn = { ...found.signIn, [method]: value }
    })
  mark({ status: "pending" })
  try {
    await ai.authenticate(connection, null, method)
    mark({ status: "done" })
    startAgentAgain(connection)
  } catch (error) {
    mark({ status: "error", message: message(error) })
  }
}

/** Forgets a connection's start; call when the connection closes. */
export function forgetAgentStart(connection: string) {
  publish(connection, (found) => {
    found.generation += 1
    found.state = IDLE
    found.request = null
    found.signIn = {}
  })
}

export function useAgentStartup(connection: string) {
  const subscribe = React.useCallback(
    (notify: () => void) => {
      const found = slot(connection)
      found.listeners.add(notify)
      return () => {
        found.listeners.delete(notify)
      }
    },
    [connection]
  )
  const state = React.useSyncExternalStore(
    subscribe,
    () => getAgentStartup(connection),
    () => getAgentStartup(connection)
  )
  const signIn = React.useSyncExternalStore(
    subscribe,
    () => getAgentSignIn(connection),
    () => getAgentSignIn(connection)
  )
  return { state, signIn }
}
