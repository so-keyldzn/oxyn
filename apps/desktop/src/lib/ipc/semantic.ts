// Mirror of `crates/oxyn-desktop/src/ipc/semantic.rs`, as executable schemas
// (ADR-0031, ADR-0056). Only the backend's reading of the option crosses: the
// webview never names a URL, a path or a file, and the switch is a workspace
// preference Rust writes through the bus as the human.

import { Channel, isTauri } from "@tauri-apps/api/core"
import { z } from "zod"

import { call, guarded, Nothing } from "./client"

export const ModelState = z.discriminatedUnion("type", [
  z.object({ type: z.literal("unavailable") }),
  z.object({ type: z.literal("absent") }),
  z.object({ type: z.literal("verifying") }),
  z.object({
    type: z.literal("downloading"),
    // Bytes of pinned files, bounded by Rust: well below 2^53, but no
    // `.int()` is needed to read them.
    received: z.number().nonnegative(),
    total: z.number().nonnegative(),
  }),
  z.object({ type: z.literal("converting") }),
  z.object({ type: z.literal("ready") }),
  z.object({ type: z.literal("corrupt") }),
  z.object({
    type: z.literal("failed"),
    /** Names no local path. Shown as text, never as markup. */
    message: z.string(),
    retryable: z.boolean(),
  }),
])
export type ModelState = z.infer<typeof ModelState>

export const SemanticSnapshot = z.object({
  enabled: z.boolean(),
  model: ModelState,
})
export type SemanticSnapshot = z.infer<typeof SemanticSnapshot>

export const semantic = {
  state: () => call("semantic_ranking_state", SemanticSnapshot),

  /** The current state first, then every change, download progress included. */
  subscribe: (onSnapshot: (snapshot: SemanticSnapshot) => void) => {
    // The channel registers a callback in Tauri's internals: constructing it
    // outside the webview throws before `call` can explain why.
    if (!isTauri()) return call("subscribe_semantic_ranking", Nothing)
    const channel = new Channel<unknown>()
    channel.onmessage = guarded(
      "subscribe_semantic_ranking",
      SemanticSnapshot,
      onSnapshot
    )
    return call("subscribe_semantic_ranking", Nothing, { channel })
  },

  /** Turns the option on and starts the download; also « Download again ». */
  enable: () => call("enable_semantic_ranking", SemanticSnapshot),

  /** Turns the option off, stopping a download and deleting the model. */
  disable: () => call("disable_semantic_ranking", SemanticSnapshot),

  /** The assistant panel opened: Rust loads the model if the option is on. */
  preload: () => call("preload_semantic_model", Nothing),
}
