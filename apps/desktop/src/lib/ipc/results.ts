// Mirror of `crates/oxyn-desktop/src/ipc/results.rs`, as executable schemas:
// both sides still change in the same commit, but a field renamed on one of
// them is now named at the boundary instead of surfacing as an `undefined`
// (ADR-0031).

import { z } from "zod"

import { call, Nothing } from "./client"
import { CommandOutcome, ResultColumn, ResultWindow } from "./types"

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
