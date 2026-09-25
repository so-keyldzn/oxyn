// Mirror of `crates/oxyn-desktop/src/ipc/results.rs`, as executable schemas:
// both sides still change in the same commit, but a field renamed on one of
// them is now named at the boundary instead of surfacing as an `undefined`
// (ADR-0031).

import { z } from "zod"

import { call, Nothing } from "./client"
import {
  CatalogAddress,
  CommandOutcome,
  ResultColumn,
  ResultWindow,
} from "./types"

/**
 * The largest window one page call returns (`MAX_PAGE_ROWS`). A page is also
 * bounded in bytes (`MAX_PAGE_BYTES`): it may hold fewer rows than asked, and
 * the reader asks again from where it stopped.
 */
export const MAX_PAGE_ROWS = 2000

export const FindAnswer = z.object({
  total: z.number().int().nonnegative(),
  row: z.number().int().nonnegative().nullable(),
  ordinal: z.number().int().nonnegative().nullable(),
  skippedBatches: z.number().int().nonnegative(),
  capped: z.boolean(),
})
export type FindAnswer = z.infer<typeof FindAnswer>

export const ValuePageView = z.object({
  column: z.string(),
  dataType: z.string(),
  isNull: z.boolean(),
  text: z.string(),
  offset: z.number().int().nonnegative(),
  nextOffset: z.number().int().nonnegative().nullable(),
  totalBytes: z.number().int().nonnegative(),
})
export type ValuePageView = z.infer<typeof ValuePageView>

/**
 * `format` is the name `export_result` accepts, not a domain format: Rust
 * sends it as a plain string, and the list holds every format the domain
 * names — including the ones `supported` says are not written yet.
 */
export const ExportFormatChoice = z.object({
  format: z.string(),
  label: z.string(),
  extension: z.string(),
  supported: z.boolean(),
})
export type ExportFormatChoice = z.infer<typeof ExportFormatChoice>

/** The most rows one copy holds (`MAX_COPY_ROWS`); refused above it. */
export const MAX_COPY_ROWS = 2000

/**
 * A « Copy rows as » format. The first four render values as the grid shows
 * them; `insert` and `inList` compose SQL in Rust, quoted and escaped by the
 * connection's dialect. None of them runs anything.
 */
export const CopyRowsFormat = z.enum([
  "tsv",
  "csv",
  "json",
  "markdown",
  "insert",
  "inList",
])
export type CopyRowsFormat = z.infer<typeof CopyRowsFormat>

export const CopiedRows = z.object({
  text: z.string(),
  rows: z.number().int().nonnegative(),
})
export type CopiedRows = z.infer<typeof CopiedRows>

/**
 * Which rows of a held result to copy, and how (`CopyRowsRequest`).
 *
 * `count` is at most `MAX_COPY_ROWS` and stays within the rows the result
 * holds; `columns` are Arrow indexes in the order the grid shows them.
 * `connection` is required for `insert` and `inList`, whose literals follow
 * its dialect, and to read rows that spilled to disk: pass it whenever it is
 * known. `address` is the relation an `insert` writes into.
 */
export const CopyRowsRequest = z.object({
  result: z.string(),
  offset: z.number().int().nonnegative(),
  count: z.number().int().nonnegative(),
  columns: z.array(z.number().int().nonnegative()),
  format: CopyRowsFormat,
  header: z.boolean(),
  connection: z.string().nullable(),
  address: CatalogAddress.nullable(),
})
export type CopyRowsRequest = z.infer<typeof CopyRowsRequest>

export const results = {
  /**
   * The hottest call in the product: the grid makes it on every scroll, for up
   * to `PAGE_CELLS` cells, inside an 8 ms frame budget. `ResultWindow` is the
   * schema measured for that — its cells are checked by shape rather than
   * parsed one by one (ADR-0031). Use it as it is; a variant declared here
   * would be both a second declaration and a second cost.
   */
  readResultPage: (
    connection: string,
    result: string,
    offset: number,
    limit: number
  ) =>
    call("read_result_page", ResultWindow, {
      connection,
      result,
      offset,
      limit,
    }),

  /** The columns of a held result, known before its first rows; `null` once expired. */
  resultColumns: (result: string) =>
    call("result_columns", z.array(ResultColumn).nullable(), { result }),

  forgetResult: (result: string) => call("forget_result", Nothing, { result }),

  exportFormats: () => call("export_formats", z.array(ExportFormatChoice)),

  /**
   * Opens the native save dialog **in Rust** and writes the file there: the
   * webview suggests a name, never a path. `null` when the user dismissed the
   * dialog. Cancellable with `cancel` under the same command id.
   */
  exportResult: (
    commandId: string,
    connection: string,
    result: string,
    format: string,
    suggestedName: string
  ) =>
    call("export_result", CommandOutcome.nullable(), {
      commandId,
      connection,
      result,
      format,
      suggestedName,
    }),

  /**
   * Rows of a held result as text for the clipboard. Reads the buffer only —
   * never the rest of the cursor, never the query again — and runs nothing.
   * Refused, never trimmed: past `MAX_COPY_ROWS`, outside the rows held, on a
   * column type with no SQL literal (the message names the column).
   */
  copyResultRows: (request: CopyRowsRequest) =>
    call("copy_result_rows", CopiedRows, { request }),

  /** `null` when the result has expired. */
  findInResult: (
    result: string,
    needle: string,
    from: number,
    forward: boolean
  ) =>
    call("find_in_result", FindAnswer.nullable(), {
      result,
      needle,
      from,
      forward,
    }),

  /** `null` when the result has expired. */
  findMatchesInWindow: (
    result: string,
    needle: string,
    offset: number,
    limit: number
  ) =>
    call(
      "find_matches_in_window",
      z.array(z.number().int().nonnegative()).nullable(),
      { result, needle, offset, limit }
    ),

  /** `null` when the result has expired. Cancel with `cancel(commandId)`. */
  inspectValue: (
    commandId: string,
    connection: string,
    result: string,
    row: number,
    column: number,
    offset: number
  ) =>
    call("inspect_value", ValuePageView.nullable(), {
      commandId,
      connection,
      result,
      row,
      column,
      offset,
    }),
}
