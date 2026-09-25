import * as React from "react"
import { createStore, useStore } from "@tanstack/react-store"

import { transactionNotice } from "@/features/consoles/console-model"
import { transactionStates } from "@/lib/ipc/events"
import type { TransactionState } from "@/lib/ipc/types"

/** A console's session, as far as its transaction goes. */
export interface HeldSession {
  session: string
  /** The session declares `TRANSACTIONS`: its state means something. */
  transactions: boolean
}

export type PendingTransaction = "open" | "unknown"

interface WorkspaceSessions {
  connection: string
  sessions: Array<HeldSession>
}

/**
 * The console sessions each retained workspace holds, by the workspace's own
 * session: a connection reopened mounts its new workspace before the old one
 * leaves. Written by the workspaces themselves — the start screen does not
 * render a hidden one, and its transaction must still be named there
 * (ADR-0046).
 */
export const workspaceSessions = createStore<Record<string, WorkspaceSessions>>(
  {}
)

export function publishWorkspaceSessions(
  workspace: string,
  connection: string,
  sessions: Array<HeldSession>
) {
  workspaceSessions.setState((all) => ({
    ...all,
    [workspace]: { connection, sessions },
  }))
}

export function forgetWorkspaceSessions(workspace: string) {
  workspaceSessions.setState((all) => {
    if (!(workspace in all)) return all
    const { [workspace]: _removed, ...rest } = all
    return rest
  })
}

/**
 * What a workspace leaves pending on its server: `open` when any console
 * reported an open transaction, `unknown` when one cannot say, else nothing.
 * The last state each session reported, never one guessed (ADR-0039 §5).
 */
export function pendingTransaction(
  sessions: Array<HeldSession>,
  states: Record<string, TransactionState>
): PendingTransaction | null {
  let pending: PendingTransaction | null = null
  for (const held of sessions) {
    const notice = transactionNotice(states[held.session], held.transactions)
    if (notice === "open") return "open"
    if (notice === "unknown") pending = "unknown"
  }
  return pending
}

/** The connections whose workspace leaves a transaction pending. */
export function usePendingTransactions(): Record<string, PendingTransaction> {
  const workspaces = useStore(workspaceSessions)
  const states = useStore(transactionStates)
  return React.useMemo(() => {
    const byConnection: Record<string, Array<HeldSession>> = {}
    for (const { connection, sessions } of Object.values(workspaces))
      byConnection[connection] = [
        ...(byConnection[connection] ?? []),
        ...sessions,
      ]
    const pending: Record<string, PendingTransaction> = {}
    for (const [connection, sessions] of Object.entries(byConnection)) {
      const state = pendingTransaction(sessions, states)
      if (state) pending[connection] = state
    }
    return pending
  }, [workspaces, states])
}
