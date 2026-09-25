// Mirror of `crates/oxyn-desktop/src/ipc/menu.rs`, as executable schemas
// (ADR-0031). The native macOS bar sends an action id and nothing else; the
// front sends back, per entry, two switches and a variant index — never a
// text or a combination (ADR-0041, point 4).

import { Channel, isTauri } from "@tauri-apps/api/core"
import { z } from "zod"

import { call, guarded, Nothing } from "./client"

export const MenuActivation = z.object({ id: z.string() })
export type MenuActivation = z.infer<typeof MenuActivation>

export const MenuEntryState = z.object({
  id: z.string(),
  enabled: z.boolean(),
  variant: z.number().int().nonnegative(),
  shortcut: z.boolean(),
})
export type MenuEntryState = z.infer<typeof MenuEntryState>

export const menu = {
  subscribe: (onActivation: (activation: MenuActivation) => void) => {
    // The channel registers a callback in Tauri's internals: constructing it
    // outside the webview throws before `call` can explain why.
    if (!isTauri()) return call("subscribe_menu", Nothing)
    const channel = new Channel<unknown>()
    channel.onmessage = guarded("subscribe_menu", MenuActivation, onActivation)
    return call("subscribe_menu", Nothing, { channel })
  },

  setState: (entries: Array<MenuEntryState>) =>
    call("set_menu_state", Nothing, { entries }),
}
