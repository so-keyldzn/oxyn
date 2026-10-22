// The editor's completion source (ADR-0057): the names of the catalog cache,
// inserted as Rust composed them.
//
// What it reads is the snapshot `completion_catalog` returned, held in memory:
// no IPC is sent on a keystroke. What it inserts is the `insert` or `qualified`
// form of the snapshot, never a quoting of its own — `@codemirror/lang-sql`
// cannot carry it (it drops an explicit `apply` inside a quoted identifier and
// does not double an embedded quote), so this source replaces its schema
// completion, and the library keeps the keywords and the aliases.
//
// Nothing here runs SQL, and nothing resolves a name to decide what to read
// from a server: the match is only used to choose what to *suggest*.

import type {
  Completion,
  CompletionContext,
  CompletionResult,
  CompletionSource,
} from "@codemirror/autocomplete"
import { ensureSyntaxTree, syntaxTree } from "@codemirror/language"
import type { EditorState, Text } from "@codemirror/state"

import type {
  CompletionCatalog,
  CompletionFields,
  CompletionNamespace,
  CompletionRelation,
} from "@/lib/ipc/completion"
import { sameName } from "@/lib/sql-identifiers"
import type { NamePart } from "@/lib/sql-identifiers"

type Tree = ReturnType<typeof syntaxTree>
type Node = ReturnType<Tree["resolveInner"]>

/** What the cursor is in, read from the syntax tree. */
interface Where {
  /** Where the text this completion replaces starts. */
  from: number
  /** The opening quote, when the cursor is inside a quoted identifier. */
  quote: string | null
  /** The qualifier typed before the cursor: `public` in `public.us|`. */
  parents: Array<NamePart>
  /** Nothing typed yet: only an explicit request completes. */
  empty: boolean
  /** The statement around the cursor. */
  statement: Node | null
}

/** A relation a statement names, with the alias it gives it. */
interface Named {
  path: Array<NamePart>
  alias: NamePart | null
}

/** Where a name was found in the snapshot. */
interface Found {
  namespace: CompletionNamespace
  relation: CompletionRelation
}

interface Index {
  /** What completes with no qualifier typed. */
  top: Array<Completion>
  /** The relations of each namespace, for after its qualifier. */
  inside: Map<CompletionNamespace, Array<Completion>>
  catalog: CompletionCatalog
}

// As `@codemirror/lang-sql` does, so the result stays valid while a name is
// being typed and is asked again when anything else is.
const SPAN = /^\w*$/
const QUOTED_SPAN = /^[`'"[]?\w*[`'"\]]?$/

// What the cursor is not completing a name in.
const NOT_A_NAME = new Set(["String", "Number", "LineComment", "BlockComment"])

// The keywords after which a relation is named, and what sits between a
// relation and the next one without ending the list.
const OPENS = new Set(["from", "join", "update", "into", "table"])
const MODIFIERS = new Set([
  "inner",
  "left",
  "right",
  "full",
  "cross",
  "natural",
  "outer",
  "lateral",
  "only",
])

/**
 * The completion source of the SQL editor.
 *
 * `read` is called on each request, so the source follows the snapshot without
 * the editor being reconfigured when a new one arrives.
 */
export function sqlCompletionSource(
  read: () => CompletionCatalog | null
): CompletionSource {
  return (context: CompletionContext): CompletionResult | null => {
    const catalog = read()
    if (catalog === null) return null
    const where = whereAt(context.state, context.pos)
    if (where === null || (where.empty && !context.explicit)) return null

    const index = indexOf(catalog)
    const named = where.statement
      ? namedRelations(context.state.doc, where.statement)
      : []
    const options =
      where.parents.length === 0
        ? [...index.top, ...columnsOf(index, named)]
        : afterQualifier(index, where.parents, named)
    if (options.length === 0) return null

    const { quote } = where
    if (quote === null) return { from: where.from, options, validFor: SPAN }
    const closing = closingQuote(quote)
    const quoteAfter =
      context.state.sliceDoc(context.pos, context.pos + 1) === closing
    return {
      from: where.from,
      to: quoteAfter ? context.pos + 1 : undefined,
      // The typed text starts with the quote: the labels must, to match it.
      // What replaces it is still the insertion form Rust composed.
      options: options.map((option) => ({
        ...option,
        label: quote + option.label + closing,
        displayLabel: option.label,
      })),
      validFor: QUOTED_SPAN,
    }
  }
}

// --- What the statement asks of the cache ------------------------------------

/**
 * The names of the statement under the cursor that the catalog cache may lack:
 * the relations it names after `FROM`, `JOIN`…, and the name being typed when
 * it is qualified — `public.us|` is `public` then `us`.
 *
 * What the editor sends the host, after a pause in typing, so that it reads
 * the namespaces and the columns these need (ADR-0057 § 2). They are names as
 * lexed, compared with the cache's by the host and never composed into a read:
 * which relation each one is, is decided there, with the resolution path the
 * server reported — the match of this file only chooses what to *suggest*.
 */
export function wantedNames(
  state: EditorState,
  pos: number
): Array<Array<NamePart>> {
  // The whole statement: a name written after the cursor still wants its
  // columns, and a tree cut short by the parser's time slice would lose it.
  ensureSyntaxTree(state, state.doc.length, 25)
  const where = whereAt(state, pos)
  if (where === null || where.statement === null) return []
  const chains = namedRelations(state.doc, where.statement).map(
    (named) => named.path
  )
  if (where.parents.length > 0) {
    const typed = state.sliceDoc(where.from, pos)
    chains.push([
      ...where.parents,
      // The unfinished last part: what is typed of it is not a name yet, and
      // the host asks for the namespace its qualifier names, not for it.
      where.quote === null
        ? { text: typed, quoted: false }
        : { text: typed.slice(1), quoted: true },
    ])
  }
  return distinct(chains)
}

function distinct(chains: Array<Array<NamePart>>): Array<Array<NamePart>> {
  const seen = new Set<string>()
  return chains.filter((chain) => {
    const key = JSON.stringify(chain)
    if (seen.has(key)) return false
    seen.add(key)
    return true
  })
}

// --- The cursor ---------------------------------------------------------------

function whereAt(state: EditorState, pos: number): Where | null {
  const tree = syntaxTree(state)
  const at = tree.resolveInner(pos, -1)
  if (NOT_A_NAME.has(at.name)) return null
  // At the very start of a statement — a console just opened, the caret at 0 —
  // the node before the caret is none, and the statement is the one after it.
  const statement =
    statementAround(at) ?? statementAround(tree.resolveInner(pos, 1))
  if (
    at.name === "Identifier" ||
    at.name === "QuotedIdentifier" ||
    at.name === "Keyword"
  ) {
    const first = at.name === "QuotedIdentifier" ? runStart(at) : at
    return {
      from: first.from,
      quote:
        at.name === "QuotedIdentifier"
          ? state.sliceDoc(first.from, first.from + 1)
          : null,
      parents: parentsOf(state.doc, tokenBefore(first)),
      empty: false,
      statement,
    }
  }
  if (at.name === ".") {
    return {
      from: pos,
      quote: null,
      parents: parentsOf(state.doc, at),
      empty: false,
      statement,
    }
  }
  return { from: pos, quote: null, parents: [], empty: true, statement }
}

function statementAround(node: Node): Node | null {
  for (let parent: Node | null = node; parent; parent = parent.parent)
    if (parent.name === "Statement") return parent
  return null
}

function tokenBefore(node: Node): Node {
  const cursor = node.cursor().moveTo(node.from, -1)
  while (/Comment/.test(cursor.name)) cursor.moveTo(cursor.from, -1)
  return cursor.node
}

function isIdentifier(node: Node | null): node is Node {
  return (
    node !== null &&
    (node.name === "Identifier" || node.name === "QuotedIdentifier")
  )
}

/** A name between identifier quotes, as typed: a doubled quote is one. */
function unquote(text: string): string {
  const opening = text[0] ?? ""
  const closing = closingQuote(opening)
  if (text.length < 2 || !text.endsWith(closing)) return text.slice(1)
  return text.slice(1, -1).replaceAll(closing + closing, closing)
}

function closingQuote(opening: string) {
  return opening === "[" ? "]" : opening
}

function partOf(doc: Text, node: Node): NamePart {
  const text = doc.sliceString(node.from, node.to)
  return node.name === "QuotedIdentifier"
    ? { text: unquote(text), quoted: true }
    : { text, quoted: false }
}

// The parser reads a doubled quote — `"a""b"`, a name holding a quote — as two
// quoted identifiers that touch. The name is the whole run, so a qualifier is
// read from its first piece, and a statement holding one is left unread.
function runStart(last: Node): Node {
  let first = last
  for (;;) {
    const before = tokenBefore(first)
    if (before.name !== "QuotedIdentifier" || before.to !== first.from)
      return first
    first = before
  }
}

function endsWithQuote(node: Node): boolean {
  return (
    node.name === "QuotedIdentifier" ||
    (node.name === "CompositeIdentifier" &&
      node.lastChild?.name === "QuotedIdentifier" &&
      node.lastChild.to === node.to)
  )
}

function startsWithQuote(node: Node): boolean {
  return (
    node.name === "QuotedIdentifier" ||
    (node.name === "CompositeIdentifier" &&
      node.firstChild?.name === "QuotedIdentifier")
  )
}

/** The dotted qualifier that ends at `dot`: `a.b.` gives `a`, `b`. */
function parentsOf(doc: Text, dot: Node): Array<NamePart> {
  const path: Array<NamePart> = []
  let node = dot
  for (;;) {
    if (node.name !== ".") return path
    const last = tokenBefore(node)
    if (!isIdentifier(last)) return path
    const first = last.name === "QuotedIdentifier" ? runStart(last) : last
    const text = doc.sliceString(first.from, last.to)
    path.unshift(
      last.name === "QuotedIdentifier"
        ? { text: unquote(text), quoted: true }
        : { text, quoted: false }
    )
    node = tokenBefore(first)
  }
}

function pathOf(doc: Text, node: Node): Array<NamePart> {
  if (node.name !== "CompositeIdentifier") return [partOf(doc, node)]
  const path: Array<NamePart> = []
  for (let child = node.firstChild; child; child = child.nextSibling)
    if (isIdentifier(child)) path.push(partOf(doc, child))
  return path
}

// --- The statement ------------------------------------------------------------

/**
 * The relations the statement names after `FROM`, `JOIN`, `UPDATE`, `INTO` and
 * `TABLE`, with their aliases — and nothing more. A subquery, a common table
 * expression or a function is not followed: what this cannot read yields no
 * column, never a guessed one.
 */
function namedRelations(doc: Text, statement: Node): Array<Named> {
  const named: Array<Named> = []
  let mode: "none" | "relation" | "alias" | "subquery" = "none"
  let list = false
  for (let scan = statement.firstChild; scan; scan = scan.nextSibling) {
    const before = scan.prevSibling
    // A name holding a quote is split by the parser: what is read of this
    // statement could name the wrong relation, so none is named.
    if (
      before &&
      before.to === scan.from &&
      endsWithQuote(before) &&
      startsWithQuote(scan)
    )
      return []
    if (scan.name === "Keyword") {
      const word = doc.sliceString(scan.from, scan.to).toLowerCase()
      if (OPENS.has(word)) {
        mode = "relation"
        list = word === "from"
      } else if (word !== "as" && !MODIFIERS.has(word)) {
        mode = "none"
        list = false
      }
      continue
    }
    if (scan.name === "Punctuation") {
      if (doc.sliceString(scan.from, scan.to) === ",")
        mode = list ? "relation" : "none"
      continue
    }
    const identifier = isIdentifier(scan)
    if (
      mode === "relation" &&
      (identifier || scan.name === "CompositeIdentifier")
    ) {
      named.push({ path: pathOf(doc, scan), alias: null })
      mode = "alias"
    } else if (mode === "relation" && scan.name === "Parens") {
      mode = "subquery"
    } else if (mode === "alias" && identifier) {
      const last = named[named.length - 1]
      if (last) last.alias = partOf(doc, scan)
      mode = "none"
    } else if (mode === "subquery" && identifier) {
      mode = "none"
    }
  }
  return named
}

// --- The snapshot -------------------------------------------------------------

const indexes = new WeakMap<CompletionCatalog, Index>()

function indexOf(catalog: CompletionCatalog): Index {
  let index = indexes.get(catalog)
  if (index === undefined) {
    index = buildIndex(catalog)
    indexes.set(catalog, index)
  }
  return index
}

function relationOption(
  relation: CompletionRelation,
  label: string,
  apply: string,
  boost: number
): Completion {
  return {
    label,
    apply,
    type: "type",
    detail: `${relation.kind.replace("_", " ")}${relation.stale ? " · stale" : ""}`,
    boost,
  }
}

function buildIndex(catalog: CompletionCatalog): Index {
  const top: Array<Completion> = []
  const inside = new Map<CompletionNamespace, Array<Completion>>()
  const bare = bareNames(catalog)
  for (const namespace of catalog.namespaces) {
    inside.set(
      namespace,
      namespace.relations.map((relation) =>
        relationOption(relation, relation.name, relation.insert, 0)
      )
    )
    if (namespace.name !== null && namespace.insert !== null) {
      top.push({
        label: namespace.name,
        apply: namespace.insert,
        type: "namespace",
        detail: namespace.system ? "system" : undefined,
        boost: namespace.system ? -2 : 0,
      })
    }
    // A system namespace is suggested only once its qualifier is typed.
    if (namespace.system) continue
    for (const relation of namespace.relations) {
      if (namespace.name === null || bare.get(namespace)?.has(relation.name))
        top.push(relationOption(relation, relation.name, relation.insert, 2))
      else
        top.push(
          relationOption(
            relation,
            `${namespace.name}.${relation.name}`,
            relation.qualified,
            -1
          )
        )
    }
  }
  return { top, inside, catalog }
}

/**
 * The relations each namespace holds that an unqualified name reaches, by the
 * resolution path the server reported.
 *
 * Nothing is bare while the path is unknown: `public` is never assumed. A
 * relation is bare only if its namespace is on the path and no earlier
 * namespace of it holds the same name — which cannot be told when an earlier
 * one was never listed, may be out of date, or when the snapshot was cut, so
 * the later ones are qualified instead.
 */
function bareNames(
  catalog: CompletionCatalog
): Map<CompletionNamespace, Set<string>> {
  const bare = new Map<CompletionNamespace, Set<string>>()
  if (catalog.resolution.state !== "known") return bare
  const shadowing = new Set<string>()
  let known = true
  catalog.resolution.namespaces.forEach((entry, position) => {
    const namespace = catalog.namespaces.find((one) => one.name === entry)
    if (namespace === undefined) {
      known = false
      return
    }
    const sure = known && (position === 0 || catalog.truncated === null)
    if (sure) {
      bare.set(
        namespace,
        new Set(
          namespace.relations
            .map((relation) => relation.name)
            .filter((name) => !shadowing.has(name))
        )
      )
    }
    for (const relation of namespace.relations) shadowing.add(relation.name)
    if (!namespace.listed || namespace.stale) known = false
  })
  return bare
}

/** The relation `path` names, or null when the snapshot cannot say which. */
function resolve(index: Index, path: Array<NamePart>): Found | null {
  const { catalog } = index
  const [first, second] = path
  if (first === undefined || path.length > 2) return null
  if (path.length === 2 && second !== undefined) {
    for (const namespace of catalog.namespaces) {
      if (!sameName(first, namespace.name)) continue
      const relation = namespace.relations.find((one) =>
        sameName(second, one.name)
      )
      if (relation) return { namespace, relation }
    }
    return null
  }
  // A source without namespaces: its relations are reached by their name.
  const rootless = catalog.namespaces.find((one) => one.name === null)
  const direct = rootless?.relations.find((one) => sameName(first, one.name))
  if (rootless && direct) return { namespace: rootless, relation: direct }
  if (catalog.resolution.state !== "known") return null
  let known = true
  for (const [position, entry] of catalog.resolution.namespaces.entries()) {
    const namespace = catalog.namespaces.find((one) => one.name === entry)
    if (namespace === undefined) return null
    const relation = namespace.relations.find((one) =>
      sameName(first, one.name)
    )
    if (relation)
      return known && (position === 0 || catalog.truncated === null)
        ? { namespace, relation }
        : null
    if (!namespace.listed || namespace.stale) known = false
  }
  return null
}

// --- What is offered ----------------------------------------------------------

function fieldOptions(fields: CompletionFields): Array<Completion> {
  if (fields.state !== "loaded") return []
  return fields.items.map((item) => ({
    label: item.name,
    apply: item.insert,
    type: "property",
    detail: item.dataType === "" ? undefined : item.dataType,
  }))
}

/** The columns of the relations the statement names, once each. */
function columnsOf(index: Index, named: Array<Named>): Array<Completion> {
  const columns = new Map<string, Completion>()
  for (const one of named) {
    const found = resolve(index, one.path)
    if (found === null) continue
    for (const option of fieldOptions(found.relation.fields)) {
      if (columns.has(option.label)) continue
      columns.set(option.label, {
        ...option,
        boost: 1,
        detail: [found.relation.name, option.detail]
          .filter(Boolean)
          .join(" · "),
      })
    }
  }
  return [...columns.values()]
}

/** What follows a qualifier: a namespace's relations, or a relation's columns. */
function afterQualifier(
  index: Index,
  parents: Array<NamePart>,
  named: Array<Named>
): Array<Completion> {
  const [only] = parents
  if (only !== undefined && parents.length === 1) {
    // An alias of the statement stands for its relation, and shadows nothing
    // else: `o.` after `FROM orders o` is the columns of `orders`.
    const aliased = named.find(
      (one) => one.alias && sameName(only, one.alias.text)
    )
    if (aliased) {
      const found = resolve(index, aliased.path)
      return found ? fieldOptions(found.relation.fields) : []
    }
  }
  const options: Array<Completion> = []
  if (only !== undefined && parents.length === 1) {
    for (const namespace of index.catalog.namespaces)
      if (sameName(only, namespace.name))
        options.push(...(index.inside.get(namespace) ?? []))
  }
  const found = resolve(index, parents)
  if (found) options.push(...fieldOptions(found.relation.fields))
  return options
}
