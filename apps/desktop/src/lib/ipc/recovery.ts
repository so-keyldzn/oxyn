// Mirror of `crates/oxyn-desktop/src/ipc/recovery.rs`, as executable schemas:
// both sides still change in the same commit, but a field renamed on one of
// them is now named at the boundary instead of surfacing as an `undefined`
// (ADR-0031).

import { Channel, isTauri } from "@tauri-apps/api/core"
import { z } from "zod"

import { call, guarded, Nothing } from "./client"
import { Environment, TransactionState } from "./types"

export const RecoveryStatus = z.object({
  /** Observed at startup: only then may the screen say Oxyn did not close normally. */
  abnormal: z.boolean(),
  /** History holds an execution the library marks `needsInspection`. */
  unresolvedWrite: z.boolean(),
})
export type RecoveryStatus = z.infer<typeof RecoveryStatus>

/** A console session whose transaction holds the exit (ADR-0043). */
export const ExitTransaction = z.object({
  session: z.string(),
  /** What `run_console` takes. Never shown: the name is. */
  connection: z.string(),
  connectionName: z.string(),
  environment: Environment,
  /** `open` or `unknown`: an idle session is not listed. */
  state: TransactionState,
})
export type ExitTransaction = z.infer<typeof ExitTransaction>

/** What an open transaction holds: the application's exit, or this window's close. */
export const ExitScope = z.enum(["application", "window"])
export type ExitScope = z.infer<typeof ExitScope>

export const ShutdownSignal = z.discriminatedUnion("type", [
  z.object({ type: z.literal("flushDrafts") }),
  z.object({
    type: z.literal("resolveTransactions"),
    /** This window's consoles only. */
    transactions: z.array(ExitTransaction),
    scope: ExitScope,
  }),
  z.object({ type: z.literal("exitCancelled") }),
])
export type ShutdownSignal = z.infer<typeof ShutdownSignal>

export const recovery = {
  status: () => call("recovery_status", RecoveryStatus),

  subscribeShutdown: (onSignal: (signal: ShutdownSignal) => void) => {
    // The channel registers a callback in Tauri's internals: constructing it
    // outside the webview throws before `call` can explain why.
    if (!isTauri()) return call("subscribe_shutdown", Nothing)
    const channel = new Channel<unknown>()
    channel.onmessage = guarded("subscribe_shutdown", ShutdownSignal, onSignal)
    return call("subscribe_shutdown", Nothing, { channel })
  },

  shutdownFlushed: () => call("shutdown_flushed", Nothing),

  /** The dialog of `resolveTransactions` or of a window close is shown: the exit waits for it. */
  shutdownAcknowledged: () => call("shutdown_acknowledged", Nothing),

  /** Abandons an exit held by a transaction, or this window's close; nothing was flushed. */
  cancelExit: () => call("cancel_exit", Nothing),

  /** The ordered exit of ⌘Q, asked by `File ▸ Exit`: takes nothing, chooses nothing. */
  requestExit: () => call("request_exit", Nothing),
}
