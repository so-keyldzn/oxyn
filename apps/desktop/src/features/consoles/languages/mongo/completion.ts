// The completion of the MongoDB console: command names and their fields,
// query and update operators, pipeline stages and accumulators — and the
// databases, collections and fields the catalog cache already holds
// (ADR-0057).
//
// What it reads of the cache is the snapshot `completion_catalog` returned,
// held in memory: no IPC on a keystroke. Nothing here decides what a command
// does; the session classifies it in Rust (ADR-0064).

import type {
  Completion,
  CompletionContext,
  CompletionResult,
} from "@codemirror/autocomplete"

import { readJson } from "./json-position"
import type { Slot, Step } from "./json-position"
import {
  ACCUMULATORS,
  COMMANDS,
  DATABASE_FIELD,
  DELETE_STATEMENT,
  INDEX_SPECIFICATION,
  QUERY_FIELD,
  QUERY_TOP,
  STAGES,
  UPDATE_OPERATORS,
  UPDATE_STATEMENT,
  VERBOSITIES,
} from "./vocabulary"
import type { CommandWord, Effect, Word } from "./vocabulary"
import type {
  CompletionCatalog,
  CompletionNamespace,
} from "@/lib/ipc/completion"

/** What is offered, before it is shaped for the place it is inserted at. */
export interface Suggestion {
  label: string
  type: string
  detail?: string
  info?: string
  boost?: number
}

/** Where in a command a container sits, read from the keys leading to it. */
type Place =
  | { kind: "command" }
  | { kind: "query" }
  | { kind: "logical" }
  | { kind: "fieldQuery" }
  | { kind: "update" }
  | { kind: "fields" }
  | { kind: "statements"; words: ReadonlyArray<Word> }
  | { kind: "statement"; words: ReadonlyArray<Word> }
  | { kind: "pipeline" }
  | { kind: "stage" }
  | { kind: "facet" }
  | { kind: "lookup" }
  | { kind: "group" }
  | { kind: "accumulator" }
  | { kind: "expression" }

/** The keys of each command whose value is a query predicate. */
const QUERY_KEYS: Partial<Record<string, string>> = {
  find: "filter",
  count: "query",
  distinct: "query",
  findAndModify: "query",
  listCollections: "filter",
}

/** The keys of each command whose value names fields. */
const FIELD_KEYS: Partial<Record<string, ReadonlyArray<string>>> = {
  find: ["sort", "projection"],
  findAndModify: ["sort", "fields"],
}

/** The keys of each command whose value is an array of statements. */
const STATEMENT_KEYS: Partial<
  Record<string, { key: string; words: ReadonlyArray<Word> }>
> = {
  update: { key: "updates", words: UPDATE_STATEMENT },
  delete: { key: "deletes", words: DELETE_STATEMENT },
  createIndexes: { key: "indexes", words: INDEX_SPECIFICATION },
}

/** Stages whose body names fields of the documents. */
const FIELD_STAGES = new Set(["$sort", "$project", "$addFields", "$set"])

/** Stages whose string value is a collection of the database. */
const COLLECTION_STAGES = new Set(["$out", "$merge", "$unionWith"])

const EFFECT: Record<Effect, string> = {
  read: "reads",
  write: "writes",
  structure: "changes structure",
}

function commandNamed(name: string | null | undefined): CommandWord | null {
  return COMMANDS.find((command) => command.name === name) ?? null
}

/** One step down from `place`, or null where nothing is completed below. */
function step(place: Place, command: string, next: Step): Place | null {
  const named = typeof next === "string"
  switch (place.kind) {
    case "command": {
      if (!named) return null
      if (QUERY_KEYS[command] === next) return { kind: "query" }
      if (FIELD_KEYS[command]?.includes(next)) return { kind: "fields" }
      const statements = STATEMENT_KEYS[command]
      if (statements?.key === next)
        return { kind: "statements", words: statements.words }
      if (command === "findAndModify" && next === "update")
        return { kind: "update" }
      if (command === "aggregate" && next === "pipeline")
        return { kind: "pipeline" }
      return null
    }
    case "statements":
      return named ? null : { kind: "statement", words: place.words }
    case "statement":
      if (next === "q" || next === "partialFilterExpression")
        return { kind: "query" }
      if (next === "u") return { kind: "update" }
      if (next === "key" || next === "sort") return { kind: "fields" }
      return null
    case "query":
      if (!named) return null
      if (next === "$and" || next === "$or" || next === "$nor")
        return { kind: "logical" }
      return next.startsWith("$") ? null : { kind: "fieldQuery" }
    case "logical":
      return named ? null : { kind: "query" }
    case "fieldQuery":
      if (next === "$not") return { kind: "fieldQuery" }
      if (next === "$elemMatch") return { kind: "query" }
      return null
    case "update":
      // An update pipeline is an array of stages.
      if (!named) return { kind: "stage" }
      return next.startsWith("$") ? { kind: "fields" } : null
    case "pipeline":
      return named ? null : { kind: "stage" }
    case "stage":
      if (next === "$match") return { kind: "query" }
      if (next === "$group") return { kind: "group" }
      if (next === "$facet") return { kind: "facet" }
      if (next === "$lookup" || next === "$unionWith") return { kind: "lookup" }
      if (named && FIELD_STAGES.has(next)) return { kind: "fields" }
      return { kind: "expression" }
    case "facet":
      return named ? { kind: "pipeline" } : null
    case "lookup":
      return next === "pipeline" ? { kind: "pipeline" } : null
    case "group":
      return next === "_id" ? { kind: "expression" } : { kind: "accumulator" }
    case "accumulator":
    case "fields":
    case "expression":
      return { kind: "expression" }
  }
}

/** The command the slot is in, and where in it: null where nothing completes. */
function locate(slot: Slot, rootKeys: Array<string>) {
  // `{"explain": {…}}`: the explained command is a command in its own right.
  const explained = slot.heads[0] === "explain" && slot.route[0] === "explain"
  const command = (explained ? slot.heads[1] : rootKeys[0]) ?? ""
  let place: Place | null = { kind: "command" }
  for (const next of slot.route.slice(explained ? 1 : 0)) {
    place = step(place, command, next)
    if (place === null) break
  }
  return { command, explained, place }
}

function words(
  list: ReadonlyArray<Word>,
  type: string,
  except: ReadonlyArray<string> = []
): Array<Suggestion> {
  return list
    .filter((word) => !except.includes(word.name))
    .map((word) => ({ label: word.name, type, info: word.info }))
}

/** The database a command runs in: its typed `$db`, or where names resolve. */
function databaseOf(
  catalog: CompletionCatalog,
  rootStrings: Map<string, string>
): CompletionNamespace | null {
  const name =
    rootStrings.get("$db") ??
    (catalog.resolution.state === "known"
      ? catalog.resolution.namespaces[0]
      : undefined)
  if (name === undefined) return null
  return catalog.namespaces.find((namespace) => namespace.name === name) ?? null
}

function collections(database: CompletionNamespace | null): Array<Suggestion> {
  return (database?.relations ?? []).map((relation) => ({
    label: relation.name,
    type: "type",
    detail: `${relation.kind.replace("_", " ")}${relation.stale ? " · stale" : ""}`,
    boost: 2,
  }))
}

function databases(catalog: CompletionCatalog): Array<Suggestion> {
  return catalog.namespaces.flatMap((namespace) =>
    namespace.name === null
      ? []
      : [
          {
            label: namespace.name,
            type: "namespace",
            detail: namespace.system ? "system" : undefined,
            boost: namespace.system ? -2 : 0,
          },
        ]
  )
}

/** The fields the cache inferred for the collection the command names. */
function fieldNames(
  database: CompletionNamespace | null,
  collection: string | undefined,
  prefix = ""
): Array<Suggestion> {
  const relation = database?.relations.find((one) => one.name === collection)
  if (relation?.fields.state !== "loaded") return []
  return relation.fields.items.map((item) => ({
    label: prefix + item.name,
    type: "property",
    detail: item.dataType === "" ? undefined : item.dataType,
    boost: 1,
  }))
}

/**
 * What completes at the slot: pure, so that the choice is tested without an
 * editor. `catalog` is null while the cache is not read: the vocabulary is
 * still offered, the names are not.
 */
export function suggestionsAt(
  text: string,
  pos: number,
  catalog: CompletionCatalog | null
): { slot: Slot; options: Array<Suggestion> } | null {
  const { slot, rootKeys, rootStrings } = readJson(text, pos)
  if (slot === null) return null
  const { command, explained, place } = locate(slot, rootKeys)
  if (place === null) return null
  const database = catalog ? databaseOf(catalog, rootStrings) : null
  // The collection is known only at the root: an explained one is not read.
  const collection = explained ? undefined : rootStrings.get(command)
  const options = ((): Array<Suggestion> => {
    if (slot.role === "key") {
      switch (place.kind) {
        case "command": {
          if (slot.position === 0)
            return COMMANDS.map((word) => ({
              label: word.name,
              type: "keyword",
              detail: EFFECT[word.effect],
              info: word.info,
            }))
          const fields = commandNamed(command)?.fields ?? []
          const own = explained ? fields : [...fields, DATABASE_FIELD]
          return words(own, "property", slot.siblings)
        }
        case "query":
          return [
            ...fieldNames(database, collection),
            ...words(QUERY_TOP, "function", slot.siblings),
          ]
        case "fieldQuery":
          return words(QUERY_FIELD, "function", slot.siblings)
        case "update":
          return words(UPDATE_OPERATORS, "function", slot.siblings)
        case "fields":
          return fieldNames(database, collection).filter(
            (field) => !slot.siblings.includes(field.label)
          )
        case "statement":
          return words(place.words, "property", slot.siblings)
        case "stage":
          return slot.position === 0 ? words(STAGES, "keyword") : []
        case "group":
          return slot.siblings.includes("_id")
            ? []
            : [{ label: "_id", type: "property", info: "The group key." }]
        case "accumulator":
          return slot.position === 0 ? words(ACCUMULATORS, "function") : []
        default:
          return []
      }
    }
    // A value.
    if (place.kind === "command") {
      if (slot.key === command && commandNamed(command)?.collection)
        return collections(database)
      if (slot.key === "$db" && !explained && catalog) return databases(catalog)
      if (slot.key === "verbosity" && command === "explain")
        return words(VERBOSITIES, "enum")
      return []
    }
    if (place.kind === "stage" && slot.key && COLLECTION_STAGES.has(slot.key))
      return collections(database)
    if (place.kind === "lookup" && slot.key === "from")
      return collections(database)
    // A field path, inside a stage: `"$amount"`.
    if (
      command === "aggregate" &&
      slot.quoted &&
      slot.typed.startsWith("$") &&
      (place.kind === "expression" ||
        place.kind === "group" ||
        place.kind === "accumulator" ||
        place.kind === "fields")
    )
      return fieldNames(database, collection, "$")
    return []
  })()
  return { slot, options }
}

/**
 * The text that replaces the slot: the name's content inside a string, a
 * string literal elsewhere, followed by a colon for a key. JSON's own
 * escaping, so a name holding a quote or a backslash cannot close the string.
 */
export function insertion(slot: Slot, label: string): string {
  const literal = JSON.stringify(label)
  if (slot.quoted) return literal.slice(1, slot.closed ? -1 : undefined)
  return slot.role === "key" ? `${literal}: ` : literal
}

/**
 * The completion source of the MongoDB console. `read` is called on each
 * request, so the source follows the snapshot without the editor being
 * reconfigured when a new one arrives.
 */
export function mongoCompletionSource(
  read: () => CompletionCatalog | null
): (context: CompletionContext) => CompletionResult | null {
  return (context) => {
    const found = suggestionsAt(
      context.state.doc.toString(),
      context.pos,
      read()
    )
    if (found === null) return null
    const { slot, options } = found
    // Between tokens, only an explicit request opens the list: a space typed
    // after a comma is not a question.
    if (!slot.quoted && slot.typed === "" && !context.explicit) return null
    if (options.length === 0) return null
    return {
      from: slot.from,
      to: slot.to,
      options: options.map((option): Completion => ({
        ...option,
        apply: insertion(slot, option.label),
      })),
      validFor: slot.quoted ? /^[^"\\\n]*$/ : /^[\w$.]*$/,
    }
  }
}
