// Mirror of `crates/oxyn-desktop/src/ipc/consoles.rs`, as executable schemas:
// both change in the same commit, and a field renamed on one side is now named
// by the validator on the first call instead of read as `undefined` (ADR-0031).

import { z } from "zod"

import { call, Nothing } from "./client"
import { CommandOutcome, ConsoleSession } from "./types"

// `ConsoleSession` is declared in `types.ts`, because `OpenConnection` carries
// one: a schema is a value, and a module cycle between values no longer
// vanishes at compile time the way `import type` did. Re-exported here, where
// the console commands that answer one live. The one name carries both the
// schema and the type it infers.
export { ConsoleSession }

/** Where a session resolves unqualified names. `namespace: null` is the server default. */
export const SessionPlace = z.object({
  catalog: z.string().nullable(),
  namespace: z.string().nullable(),
})
export type SessionPlace = z.infer<typeof SessionPlace>

export const ContextOutcome = z.discriminatedUnion("type", [
  z.object({ type: z.literal("set"), place: SessionPlace.nullable() }),
  z.object({ type: z.literal("cancelled") }),
])
export type ContextOutcome = z.infer<typeof ContextOutcome>

/** Offsets are UTF-16 code units, as the editor counts them. */
export const RunTarget = z.discriminatedUnion("kind", [
  z.object({ kind: z.literal("all") }),
  z.object({
    kind: z.literal("selection"),
    start: z.number().int().nonnegative(),
    end: z.number().int().nonnegative(),
  }),
  z.object({
    kind: z.literal("statement"),
    cursor: z.number().int().nonnegative(),
  }),
])
export type RunTarget = z.infer<typeof RunTarget>

export const ParameterKind = z.enum([
  "null",
  "bool",
  "int64",
  "float64",
  "decimal",
  "text",
  "bytes",
  "uuid",
  "date",
  "time",
  "timestamp",
  "timestampNaive",
  "json",
])
export type ParameterKind = z.infer<typeof ParameterKind>

/**
 * Why a bound value was refused: a position and a type, never the text
 * (I-03). `position` is `null` when no single row is at fault — that is the
 * case when the *count* of parameters is refused, not the value at any one
 * of them.
 */
export const ParameterRefusal = z.object({
  position: z.number().int().positive().nullable(),
  expectedType: ParameterKind.nullable(),
  message: z.string(),
})
export type ParameterRefusal = z.infer<typeof ParameterRefusal>

/** In picker order, with the labels `oxyn_core::ParameterType::label` uses. */
export const PARAMETER_KINDS: ReadonlyArray<{
  kind: ParameterKind
  label: string
}> = [
  { kind: "null", label: "NULL" },
  { kind: "bool", label: "Boolean" },
  { kind: "int64", label: "Int64" },
  { kind: "float64", label: "Float64" },
  { kind: "decimal", label: "Decimal" },
  { kind: "text", label: "Text" },
  { kind: "bytes", label: "Bytes (hex)" },
  { kind: "uuid", label: "UUID" },
  { kind: "date", label: "Date" },
  { kind: "time", label: "Time" },
  { kind: "timestamp", label: "Timestamp with timezone" },
  { kind: "timestampNaive", label: "Timestamp without timezone" },
  { kind: "json", label: "JSON" },
]

/** The most values one run binds; the backend enforces it too. */
export const MAX_PARAMETERS = 128

/**
 * A positional bound value. Never logged, never saved with the query (I-03).
 *
 * `type` is the JSON name of the Rust field `kind`, which is a plain field
 * here and not a tag: `ParameterInput` is a struct, not a union.
 */
export const ParameterInput = z.object({
  type: ParameterKind,
  text: z.string(),
})
export type ParameterInput = z.infer<typeof ParameterInput>

export const ConsoleRun = z.object({
  sql: z.string(),
  target: RunTarget,
  parameters: z.array(ParameterInput),
  explain: z.boolean(),
})
export type ConsoleRun = z.infer<typeof ConsoleRun>

export const consoles = {
  open: (commandId: string, connection: string) =>
    call("open_console", ConsoleSession, { commandId, connection }),

  close: (connection: string, session: string) =>
    call("close_console", Nothing, { connection, session }),

  run: (
    commandId: string,
    connection: string,
    session: string,
    run: ConsoleRun
  ) =>
    call("run_console", CommandOutcome, {
      commandId,
      connection,
      session,
      run,
    }),

  setContext: (
    commandId: string,
    connection: string,
    session: string,
    place: SessionPlace
  ) =>
    call("set_session_context", ContextOutcome, {
      commandId,
      connection,
      session,
      place,
    }),

  /** `null` when every value converts; otherwise a refusal naming a position and a type. */
  validateParameters: (parameters: Array<ParameterInput>) =>
    call("validate_parameters", ParameterRefusal.nullable(), { parameters }),

  contextChoices: (connection: string) =>
    call("session_context_choices", z.array(SessionPlace), { connection }),
}
