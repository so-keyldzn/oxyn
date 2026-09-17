import { Channel, invoke, isTauri } from "@tauri-apps/api/core"
import { z } from "zod"

import {
  CatalogNode,
  CommandOutcome,
  ConnectResponse,
  DriverChoice,
  ExecutionEvent,
  IpcError,
  OpenConnection,
  RelationDetail,
} from "./types"

import type { CatalogAddress, ConnectionDraft } from "./types"

// Every call to the backend goes through `call`, and every function built on
// it maps to exactly one `#[tauri::command]` in `crates/oxyn-desktop/src/commands`.
// Feature modules of `src/lib/ipc/` (one per feature, next to this file) use
// `call` too. Views never call `invoke` themselves: a second path to the
// backend would be the one nobody audits (I-01).
//
// Being the single path, `call` is also the single place the answer is
// checked: it takes a schema, not a type parameter. `invoke<T>` only casts,
// which is why a field renamed on one side used to surface as an `undefined`
// three screens away instead of an error (ADR-0031).

export class BackendError extends Error {
  readonly retryable: boolean

  constructor(error: IpcError) {
    super(error.message)
    this.name = "BackendError"
    this.retryable = error.retryable
  }
}

/**
 * A rejection from a command, read with the schema rather than sniffed.
 *
 * The hand-written guard this replaces asserted `IpcError` after looking at
 * `message` alone, leaving `retryable` unchecked — the one field whose
 * misreading has a consequence, since it draws the **Retry** button over an
 * operation that may already have been applied (I-13). Anything else becomes a
 * non-retryable error, which is the conservative reading.
 */
function asIpcError(value: unknown): IpcError | null {
  const parsed = IpcError.safeParse(value)
  return parsed.success ? parsed.data : null
}

/** A command that answers nothing. `invoke` resolves it as `null`, not `undefined`. */
export const Nothing = z.unknown().transform(() => undefined as void)

export async function call<T>(
  command: string,
  schema: z.ZodType<T>,
  args?: Record<string, unknown>
): Promise<T> {
  if (!isTauri()) {
    // Storybook and a plain browser have no backend. Saying so beats a
    // `window.__TAURI_INTERNALS__ is undefined` nobody can act on.
    throw new BackendError({
      message: `The Oxyn backend is not available here (${command}): open the desktop application.`,
      retryable: false,
    })
  }
  let answer: unknown
  try {
    answer = await invoke(command, args)
  } catch (error) {
    const failure = asIpcError(error)
    if (failure) throw new BackendError(failure)
    throw new BackendError({ message: String(error), retryable: false })
  }
  const parsed = schema.safeParse(answer)
  if (!parsed.success) {
    // Not retryable: a malformed answer is a mismatch between this file and
    // `ipc.rs`, and it will be just as malformed on a second call (I-13).
    throw new BackendError({
      message: `The backend answered ${command} with something this version cannot read: ${describe(parsed.error)}.`,
      retryable: false,
    })
  }
  return parsed.data
}

/** Names the offending fields, and stops well short of dumping the payload (I-03). */
function describe(error: z.ZodError): string {
  const issues = error.issues.slice(0, 3).map((issue) => {
    const path = issue.path.join(".")
    return path ? `${path}: ${issue.message}` : issue.message
  })
  const rest = error.issues.length - issues.length
  return rest > 0 ? `${issues.join("; ")} (+${rest} more)` : issues.join("; ")
}

/** A fresh id the front gives a command, so it can cancel it later. */
export function newCommandId(): string {
  return crypto.randomUUID()
}

/**
 * Guards a `Channel`, which `call` cannot reach.
 *
 * A channel message crosses the same boundary as a response, but arrives
 * later and outside any call. An unreadable one is dropped with a console
 * error rather than thrown: nothing is awaiting it, and a throw inside
 * `onmessage` would be swallowed by Tauri's internals.
 */
export function guarded<T>(
  stream: string,
  schema: z.ZodType<T>,
  onMessage: (message: T) => void
): (raw: unknown) => void {
  return (raw) => {
    const parsed = schema.safeParse(raw)
    if (!parsed.success) {
      console.error(
        `Dropped a ${stream} message this version cannot read: ${describe(parsed.error)}`
      )
      return
    }
    onMessage(parsed.data)
  }
}

export const backend = {
  listDrivers: () => call("list_drivers", z.array(DriverChoice)),

  // `listConnections` lives in `lib/ipc/settings.ts`, with the rest of the
  // saved-connection commands: importing its schema here would close a cycle
  // between the two files (see the note there).

  connect: (commandId: string, draft: ConnectionDraft) =>
    call("connect", ConnectResponse, { commandId, draft }),

  decideConnection: (command: string, approved: boolean) =>
    call("decide_connection", ConnectResponse.nullable(), {
      command,
      approved,
    }),

  reconnect: (commandId: string, connection: string) =>
    call("reconnect", OpenConnection, { commandId, connection }),

  disconnect: (connection: string) =>
    call("disconnect", CommandOutcome, { connection }),

  execute: (
    commandId: string,
    connection: string,
    session: string,
    sql: string
  ) => call("execute", CommandOutcome, { commandId, connection, session, sql }),

  previewRelation: (
    commandId: string,
    connection: string,
    session: string,
    address: CatalogAddress
  ) =>
    call("preview_relation", CommandOutcome, {
      commandId,
      connection,
      session,
      address,
    }),

  refreshCatalog: (
    commandId: string,
    connection: string,
    session: string,
    address: CatalogAddress | null
  ) =>
    call("refresh_catalog", CommandOutcome, {
      commandId,
      connection,
      session,
      address,
    }),

  catalogTree: (connection: string) =>
    call("catalog_tree", z.array(CatalogNode), { connection }),

  relationDetail: (connection: string, address: CatalogAddress) =>
    call("relation_detail", RelationDetail.nullable(), {
      connection,
      address,
    }),

  decide: (command: string, approved: boolean) =>
    call("decide", CommandOutcome, { command, approved }),

  cancel: (commandId: string) => call("cancel", z.boolean(), { commandId }),

  forgetResult: (result: string) => call("forget_result", Nothing, { result }),

  // An export lives in `lib/ipc/results.ts`: the destination is chosen by the
  // save dialog Rust opens, and no path crosses the boundary any more
  // (docs/SECURITY.md, « Surface d'entrée »).

  subscribeEvents: (onEvent: (event: ExecutionEvent) => void) => {
    // The channel registers a callback in Tauri's internals: constructing it
    // outside the webview throws before `call` can explain why.
    if (!isTauri()) return call("subscribe_events", Nothing)
    const channel = new Channel<unknown>()
    channel.onmessage = guarded("subscribe_events", ExecutionEvent, onEvent)
    return call("subscribe_events", Nothing, { channel })
  },
}

export type Backend = typeof backend
