// Mirror of `crates/oxyn-desktop/src/ipc/file_drops.rs`, as executable schemas
// (ADR-0031). Rust classified every dropped file before it reaches here: the
// front receives a `.sql` file's text, or a database file's path as a form
// value to submit — never a path any command would read on its asking.

import { Channel, isTauri } from "@tauri-apps/api/core"
import { z } from "zod"

import { call, guarded, Nothing } from "./client"

export const DroppedFile = z.discriminatedUnion("type", [
  z.object({ type: z.literal("sql"), name: z.string(), text: z.string() }),
  z.object({
    type: z.literal("database"),
    name: z.string(),
    driver: z.string(),
    field: z.string(),
    path: z.string(),
  }),
  z.object({
    type: z.literal("refused"),
    name: z.string(),
    reason: z.string(),
  }),
])
export type DroppedFile = z.infer<typeof DroppedFile>

export const fileDrops = {
  subscribe: (onDrop: (file: DroppedFile) => void) => {
    // Constructing a channel outside the webview throws before `call` can
    // explain why.
    if (!isTauri()) return call("subscribe_file_drops", Nothing)
    const channel = new Channel<unknown>()
    channel.onmessage = guarded("subscribe_file_drops", DroppedFile, onDrop)
    return call("subscribe_file_drops", Nothing, { channel })
  },
}
