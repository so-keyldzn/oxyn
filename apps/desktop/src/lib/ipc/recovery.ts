// Mirror of `crates/oxyn-desktop/src/ipc/recovery.rs`, as executable schemas:
// both sides still change in the same commit, but a field renamed on one of
// them is now named at the boundary instead of surfacing as an `undefined`
// (ADR-0031).

import { Channel, isTauri } from "@tauri-apps/api/core"
import { z } from "zod"

import { call, guarded, Nothing } from "./client"

export const RecoveryStatus = z.object({
  /** Observed at startup: only then may the screen say Oxyn did not close normally. */
  abnormal: z.boolean(),
})
export type RecoveryStatus = z.infer<typeof RecoveryStatus>

export const ShutdownSignal = z.object({ type: z.literal("flushDrafts") })
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
}
