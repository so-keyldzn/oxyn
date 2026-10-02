// Mirror of `crates/oxyn-desktop/src/ipc/updates.rs`, as executable schemas
// (ADR-0031, ADR-0051). Only the backend's reading of the update crosses: the
// webview never sends a URL, a path or a version, and never reaches the
// updater plugin itself.

import { Channel, isTauri } from "@tauri-apps/api/core"
import { z } from "zod"

import { call, guarded, Nothing } from "./client"

export const UpdateErrorKind = z.enum([
  "offline",
  "server",
  "signature",
  "install",
])
export type UpdateErrorKind = z.infer<typeof UpdateErrorKind>

export const DisabledReason = z.enum(["user", "admin", "packageManager", "dev"])
export type DisabledReason = z.infer<typeof DisabledReason>

export const UpdateState = z.discriminatedUnion("type", [
  z.object({ type: z.literal("idle") }),
  z.object({ type: z.literal("checking") }),
  z.object({ type: z.literal("upToDate"), checkedAt: z.string() }),
  z.object({ type: z.literal("available"), version: z.string() }),
  z.object({
    type: z.literal("downloading"),
    version: z.string(),
    // What the server reports: no `.int()`, which would refuse an honest
    // size beyond 2^53 (front.md).
    received: z.number().nonnegative(),
    total: z.number().nonnegative().nullable(),
  }),
  z.object({
    type: z.literal("ready"),
    version: z.string(),
    /** Plain text, at most 4 KiB. Never rendered as HTML. */
    notes: z.string().nullable(),
    date: z.string().nullable(),
    readyAt: z.string(),
    installOnQuit: z.boolean(),
  }),
  z.object({
    type: z.literal("error"),
    kind: UpdateErrorKind,
    message: z.string(),
    retryable: z.boolean(),
    version: z.string().nullable(),
  }),
  z.object({ type: z.literal("disabled"), reason: DisabledReason }),
])
export type UpdateState = z.infer<typeof UpdateState>

export const UpdateSnapshot = z.object({
  state: UpdateState,
  currentVersion: z.string(),
  automatic: z.boolean(),
  lastCheckedAt: z.string().nullable(),
})
export type UpdateSnapshot = z.infer<typeof UpdateSnapshot>

export const RestartOutcome = z.discriminatedUnion("type", [
  z.object({ type: z.literal("started") }),
  z.object({
    type: z.literal("busy"),
    running: z.number().int().nonnegative(),
    exports: z.number().int().nonnegative(),
  }),
])
export type RestartOutcome = z.infer<typeof RestartOutcome>

export const UpdateNotice = z.discriminatedUnion("type", [
  z.object({ type: z.literal("installed"), from: z.string(), to: z.string() }),
  z.object({
    type: z.literal("installFailed"),
    version: z.string(),
    message: z.string(),
  }),
])
export type UpdateNotice = z.infer<typeof UpdateNotice>

export const updates = {
  state: () => call("get_update_state", UpdateSnapshot),

  subscribe: (onSnapshot: (snapshot: UpdateSnapshot) => void) => {
    // The channel registers a callback in Tauri's internals: constructing it
    // outside the webview throws before `call` can explain why.
    if (!isTauri()) return call("subscribe_updates", Nothing)
    const channel = new Channel<unknown>()
    channel.onmessage = guarded("subscribe_updates", UpdateSnapshot, onSnapshot)
    return call("subscribe_updates", Nothing, { channel })
  },

  check: () => call("check_for_updates", Nothing),

  /** Aborts the check or the download under way. */
  cancel: () => call("cancel_update", Nothing),

  download: () => call("download_update", Nothing),

  /**
   * `busy` asks first; `confirmed` stops the running work. An open
   * transaction still holds the exit, with the `restart` scope.
   */
  restartToUpdate: ({ confirmed }: { confirmed: boolean }) =>
    call("restart_to_update", RestartOutcome, { confirmed }),

  setAutomatic: (enabled: boolean) =>
    call("set_automatic_updates", Nothing, { enabled }),

  /** The URL is built in Rust: the webview names no address. */
  openReleasePage: () => call("open_release_page", Nothing),

  /** Read once: a second call answers `null`. */
  takeNotice: () => call("take_update_notice", UpdateNotice.nullable()),
}
