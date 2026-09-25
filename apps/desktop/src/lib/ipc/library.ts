// Mirror of `crates/oxyn-desktop/src/ipc/library.rs`, as executable schemas:
// both sides still change in the same commit, but a field renamed on one side
// only is now named at the boundary instead of read as `undefined` (ADR-0031).

import { z } from "zod"

import { call, Nothing } from "./client"
import { ResultColumn } from "./types"

export const DocumentChange = z.object({
  document: z.string(),
  /** Monotonic per document; the backend refuses one that does not advance. */
  revision: z.number().int().nonnegative(),
  title: z.string(),
  text: z.string(),
  connection: z.string().nullable(),
  named: z.boolean(),
})
export type DocumentChange = z.infer<typeof DocumentChange>

export const DocumentWrite = z.discriminatedUnion("type", [
  z.object({
    type: z.literal("saved"),
    revision: z.number().int().nonnegative(),
    savedRevision: z.number().int().nonnegative(),
    isSaved: z.boolean(),
  }),
  z.object({ type: z.literal("closed") }),
  /** The stored document moved: keep the local text, offer a new copy. */
  z.object({ type: z.literal("conflict"), message: z.string() }),
  /** A newer draft replaced this one before it was written. */
  z.object({ type: z.literal("superseded") }),
])
export type DocumentWrite = z.infer<typeof DocumentWrite>

export const DocumentView = z.object({
  id: z.string(),
  title: z.string(),
  text: z.string(),
  savedTitle: z.string().nullable(),
  savedText: z.string().nullable(),
  revision: z.number().int().nonnegative(),
  savedRevision: z.number().int().nonnegative(),
  isSaved: z.boolean(),
  isOpen: z.boolean(),
  connection: z.string().nullable(),
  fromAgent: z.boolean(),
})
export type DocumentView = z.infer<typeof DocumentView>

export const DocumentQuery = z.object({
  savedOnly: z.boolean(),
  openOnly: z.boolean(),
  search: z.string(),
  before: z.string().nullable(),
  limit: z.number().int().nonnegative().nullable(),
})
export type DocumentQuery = z.infer<typeof DocumentQuery>

export const DocumentEntry = z.object({
  id: z.string(),
  title: z.string(),
  connection: z.string().nullable(),
  connectionName: z.string().nullable(),
  /** RFC 3339, UTC: the backend formats it, the front never reparses it. */
  updatedAt: z.string(),
  isSaved: z.boolean(),
  isOpen: z.boolean(),
  hasChanges: z.boolean(),
  fromAgent: z.boolean(),
})
export type DocumentEntry = z.infer<typeof DocumentEntry>

export const DocumentList = z.object({
  entries: z.array(DocumentEntry),
  next: z.string().nullable(),
})
export type DocumentList = z.infer<typeof DocumentList>

export const HistoryStatusChoice = z.enum([
  "all",
  "running",
  "succeeded",
  "failed",
  "cancelled",
  "denied",
  "ambiguous",
])
export type HistoryStatusChoice = z.infer<typeof HistoryStatusChoice>

export const HistoryQuery = z.object({
  connection: z.string().nullable(),
  search: z.string(),
  /** `null` means all dates. */
  days: z.number().int().nonnegative().nullable(),
  status: HistoryStatusChoice,
  before: z.number().int().nullable(),
  limit: z.number().int().nonnegative().nullable(),
})
export type HistoryQuery = z.infer<typeof HistoryQuery>

export const HistoryRow = z.object({
  id: z.number().int(),
  /** RFC 3339, UTC: the backend formats it, the front never reparses it. */
  at: z.string(),
  connectionName: z.string().nullable(),
  /** At most 256 characters; never executed. */
  preview: z.string(),
  status: z.string(),
  durationMs: z.number().nonnegative().nullable(),
  rows: z.number().int().nonnegative().nullable(),
  needsInspection: z.boolean(),
  /** The user declared this write inspected on the server; it no longer warns. */
  reconciled: z.boolean(),
  /** The connection it ran on, to address a retained result. Never rendered. */
  connection: z.string().nullable(),
  /** A result that may still be retained; its buffer can have expired. */
  result: z.string().nullable(),
  /** Submitted by an agent: a copy keeps saying so (ADR-0023). */
  fromAgent: z.boolean(),
})
export type HistoryRow = z.infer<typeof HistoryRow>

export const HistoryList = z.object({
  entries: z.array(HistoryRow),
  next: z.number().int().nullable(),
})
export type HistoryList = z.infer<typeof HistoryList>

export const HistoryDetail = z.object({
  id: z.number().int(),
  statement: z.string(),
  connectionName: z.string().nullable(),
  status: z.string(),
  error: z.string().nullable(),
  needsInspection: z.boolean(),
  /** Submitted by an agent: the console it is copied into says so. */
  fromAgent: z.boolean(),
})
export type HistoryDetail = z.infer<typeof HistoryDetail>

export const HistoryConnection = z.object({
  connection: z.string(),
  name: z.string().nullable(),
  inWorkspace: z.boolean(),
})
export type HistoryConnection = z.infer<typeof HistoryConnection>

export const HistoryConnectionList = z.object({
  entries: z.array(HistoryConnection),
  next: z.number().int().nullable(),
})
export type HistoryConnectionList = z.infer<typeof HistoryConnectionList>

export const RetainedResult = z.discriminatedUnion("type", [
  z.object({
    type: z.literal("open"),
    result: z.string(),
    columns: z.array(ResultColumn),
    rows: z.number().int().nonnegative(),
    complete: z.boolean(),
    /** The buffer stopped retaining rows at its bound. */
    truncated: z.boolean(),
  }),
  /** Released: nothing is rerun to bring it back. */
  z.object({ type: z.literal("expired") }),
])
export type RetainedResult = z.infer<typeof RetainedResult>

export const library = {
  /** Minted by the backend: library pages order documents by identifier age. */
  newDocument: () => call("new_query_document", z.string()),

  /** `commandId` lets `backend.cancel` stop the write before it commits. */
  saveDocument: (commandId: string, change: DocumentChange) =>
    call("save_query_document", DocumentWrite, { commandId, change }),

  /** `commandId` lets `backend.cancel` stop the close before it commits. */
  closeDocument: (
    commandId: string,
    document: string,
    revision: number,
    discard: boolean
  ) =>
    call("close_query_document", DocumentWrite, {
      commandId,
      document,
      revision,
      discard,
    }),

  releaseDocument: (document: string) =>
    call("release_query_document", Nothing, { document }),

  deleteDocument: (document: string, revision: number) =>
    call("delete_query_document", Nothing, { document, revision }),

  openDocument: (document: string) =>
    call("open_query_document", DocumentView, { document }),

  listDocuments: (commandId: string, query: DocumentQuery) =>
    call("list_query_documents", DocumentList, { commandId, query }),

  readHistory: (commandId: string, query: HistoryQuery) =>
    call("read_history", HistoryList, { commandId, query }),

  readHistoryEntry: (entry: number) =>
    call("read_history_entry", HistoryDetail, { entry }),

  /** Declares an unresolved write inspected on the server. Retries nothing. */
  reconcileHistoryEntry: (entry: number) =>
    call("reconcile_history_entry", Nothing, { entry }),

  /** Reopens a retained buffer: no query, no session. */
  openRetainedResult: (connection: string, result: string) =>
    call("open_retained_result", RetainedResult, { connection, result }),

  historyConnections: (before: number | null) =>
    call("list_history_connections", HistoryConnectionList, { before }),
}
