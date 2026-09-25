// Mirror of `crates/oxyn-desktop/src/ipc/windows.rs`, as executable schemas
// (ADR-0031), and of the window commands of `commands/windows.rs` (ADR-0043).
// No command takes a window label: Rust knows which window calls.

import { Channel, isTauri } from "@tauri-apps/api/core"
import { z } from "zod"

import { Nothing, call, guarded } from "./client"

export const WindowSignal = z.discriminatedUnion("type", [
  /** Close asked on this window, not the last, and no transaction holds it. */
  z.object({ type: z.literal("closeRequested") }),
  /** A saved connection this window holds was edited: read it again. */
  z.object({ type: z.literal("connectionChanged"), connection: z.string() }),
  /** Another window wrote the preferences: read them again. */
  z.object({ type: z.literal("preferencesChanged") }),
])
export type WindowSignal = z.infer<typeof WindowSignal>

export const windows = {
  /** `New window`: an empty window on the connection screen, 16 at most. */
  open: () => call("open_window", Nothing),

  subscribe: (onSignal: (signal: WindowSignal) => void) => {
    // The channel registers a callback in Tauri's internals: constructing it
    // outside the webview throws before `call` can explain why.
    if (!isTauri()) return call("subscribe_window", Nothing)
    const channel = new Channel<unknown>()
    channel.onmessage = guarded("subscribe_window", WindowSignal, onSignal)
    return call("subscribe_window", Nothing, { channel })
  },

  /** Asks again for this window's close, its transactions resolved. */
  close: () => call("close_window", Nothing),

  /** This window's consoles are closed: it may go. */
  confirmClose: () => call("confirm_window_close", Nothing),
}
