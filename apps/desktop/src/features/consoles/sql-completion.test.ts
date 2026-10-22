import { CompletionContext } from "@codemirror/autocomplete"
import { MySQL, PostgreSQL, SQLite, sql } from "@codemirror/lang-sql"
import { EditorState } from "@codemirror/state"
import { describe, expect, it } from "vitest"

import { sqlCompletionSource, wantedNames } from "./sql-completion"
import type {
  CompletionCatalog,
  CompletionFields,
  CompletionNamespace,
  CompletionRelation,
} from "@/lib/ipc/completion"

const HOSTILE = 'users"; DROP TABLE audit; --'
// What Rust composes for it: the quote is doubled, so it cannot close early.
const HOSTILE_INSERT = '"users""; DROP TABLE audit; --"'

function loaded(...columns: Array<[string, string]>): CompletionFields {
  return {
    state: "loaded",
    items: columns.map(([name, dataType]) => ({
      name,
      insert: /^[a-z_][a-z0-9_]*$/.test(name)
        ? name
        : `"${name.replaceAll('"', '""')}"`,
      dataType,
    })),
  }
}

function relation(
  name: string,
  fields: CompletionFields = { state: "notLoaded" },
  extra: Partial<CompletionRelation> = {}
): CompletionRelation {
  const insert = /^[a-z_][a-z0-9_]*$/.test(name)
    ? name
    : `"${name.replaceAll('"', '""')}"`
  return {
    name,
    insert,
    qualified: insert,
    kind: "table",
    stale: false,
    virtualTable: false,
    shadow: false,
    fields,
    ...extra,
  }
}

function namespace(
  name: string | null,
  relations: Array<CompletionRelation>,
  extra: Partial<CompletionNamespace> = {}
): CompletionNamespace {
  return {
    name,
    insert: name,
    system: false,
    stale: false,
    listed: true,
    // What Rust composes: the namespace's form before the relation's.
    relations: relations.map((one) => ({
      ...one,
      qualified: name === null ? one.insert : `${name}.${one.insert}`,
    })),
    ...extra,
  }
}

function snapshot(
  namespaces: Array<CompletionNamespace>,
  path: Array<string> | null = ["public"],
  truncated: CompletionCatalog["truncated"] = null
): CompletionCatalog {
  return {
    namespaces,
    resolution:
      path === null
        ? { state: "unknown" }
        : { state: "known", namespaces: path, stale: false },
    truncated,
  }
}

const SHOP = snapshot([
  namespace("public", [
    relation(
      "users",
      loaded(["id", "int8"], ["name", "text"], ["Full Name", "text"])
    ),
    relation("orders", loaded(["id", "int8"], ["user_id", "int8"])),
    // A reserved word: Rust composes it quoted.
    relation("order", { state: "notLoaded" }, { insert: '"order"' }),
    relation(HOSTILE),
    relation("Order Lines", loaded(["qty", "int4"])),
    relation('a"b', loaded(["x", "int4"])),
  ]),
  namespace("archive", [relation("users", loaded(["old_id", "int8"]))]),
  namespace("pg_catalog", [relation("pg_class")], { system: true }),
])

type Dialect = typeof PostgreSQL

/** Completes at the `|` of `text`, as the editor asks on a keystroke. */
function complete(
  text: string,
  catalog: CompletionCatalog | null = SHOP,
  dialect: Dialect = PostgreSQL,
  explicit = false
) {
  const pos = text.indexOf("|")
  const doc = text.replace("|", "")
  const state = EditorState.create({ doc, extensions: [sql({ dialect })] })
  const source = sqlCompletionSource(() => catalog)
  const result = source(new CompletionContext(state, pos, explicit))
  if (result instanceof Promise) throw new Error("the source is synchronous")
  return result
}

function applied(text: string, catalog: CompletionCatalog | null = SHOP) {
  const result = complete(text, catalog)
  return (result?.options ?? []).map((option) => ({
    label: option.label,
    apply: option.apply,
  }))
}

describe("what completes with no qualifier", () => {
  it("offers the relations the path reaches, bare, and the others qualified", () => {
    const options = applied("SELECT * FROM us|")
    expect(options).toContainEqual({ label: "users", apply: "users" })
    // `archive` is not on the path: only its qualified form is suggested.
    expect(options).toContainEqual({
      label: "archive.users",
      apply: "archive.users",
    })
  })

  it("offers a namespace by name, so that its qualifier can be typed", () => {
    const options = applied("SELECT * FROM arc|")
    expect(options).toContainEqual({ label: "archive", apply: "archive" })
  })

  it("inserts what Rust composed, never a quoting of its own", () => {
    const result = complete("SELECT * FROM |", SHOP, PostgreSQL, true)
    const insertion = (label: string) =>
      result?.options.find((option) => option.label === label)?.apply
    // The quote inside is doubled, a reserved word is quoted: not by this code.
    expect(insertion(HOSTILE)).toBe(HOSTILE_INSERT)
    expect(insertion("order")).toBe('"order"')
    expect(insertion("Order Lines")).toBe('"Order Lines"')
  })

  it("never suggests what the cursor is not naming: strings and comments", () => {
    expect(complete("SELECT 'us|'")).toBeNull()
    expect(complete("-- us|")).toBeNull()
    expect(complete("SELECT 12|")).toBeNull()
  })

  it("waits for a character, unless asked", () => {
    expect(complete("SELECT * FROM |")).toBeNull()
    expect(complete("SELECT * FROM |", SHOP, PostgreSQL, true)).not.toBeNull()
  })

  it("says nothing without a snapshot", () => {
    expect(complete("SELECT * FROM us|", null)).toBeNull()
  })

  it("does not offer the relations of a system namespace before its qualifier", () => {
    const labels = applied("SELECT * FROM pg|").map((option) => option.label)
    expect(labels).toContain("pg_catalog")
    expect(labels).not.toContain("pg_class")
    expect(labels).not.toContain("pg_catalog.pg_class")
    expect(applied("SELECT * FROM pg_catalog.pg|")).toContainEqual({
      label: "pg_class",
      apply: "pg_class",
    })
  })
})

describe("the resolution path", () => {
  it("suggests qualified names only while the path is unknown", () => {
    const options = applied(
      "SELECT * FROM us|",
      snapshot(SHOP.namespaces, null)
    )
    expect(options.map((option) => option.label)).toEqual(
      expect.arrayContaining(["public.users", "archive.users"])
    )
    expect(options.map((option) => option.label)).not.toContain("users")
  })

  it("suggests a name bare only in the first namespace of the path that holds it", () => {
    const both = snapshot(SHOP.namespaces, ["archive", "public"])
    const labels = applied("SELECT * FROM us|", both).map((o) => o.label)
    expect(labels).toContain("users")
    expect(labels).toContain("public.users")
    expect(labels.filter((label) => label === "users")).toHaveLength(1)
    expect(
      applied("SELECT * FROM us|", both).find((o) => o.label === "users")
    ).toEqual({ label: "users", apply: "users" })
  })

  it("qualifies what follows a namespace nobody read, which may shadow it", () => {
    const unread = snapshot(
      [
        namespace("tenant", [], { listed: false }),
        namespace("public", [relation("users")]),
      ],
      ["tenant", "public"]
    )
    const labels = applied("SELECT * FROM us|", unread).map((o) => o.label)
    expect(labels).toContain("public.users")
    expect(labels).not.toContain("users")
  })

  it("qualifies what follows a namespace that may be out of date", () => {
    const stale = snapshot(
      [
        namespace("tenant", [relation("a")], { stale: true }),
        namespace("public", [relation("users")]),
      ],
      ["tenant", "public"]
    )
    expect(applied("SELECT * FROM us|", stale).map((o) => o.label)).toContain(
      "public.users"
    )
  })

  it("qualifies after the first namespace when the snapshot was cut", () => {
    const cut = snapshot(
      [namespace("a", [relation("x")]), namespace("b", [relation("y")])],
      ["a", "b"],
      { namespacesLeftOut: 0, relationsLeftOut: 3, fieldsLeftOut: 0 }
    )
    const all = complete("SELECT * FROM |", cut, PostgreSQL, true)
    const names = all?.options.map((option) => option.label) ?? []
    // `a` comes first: nothing shadows it. `b` follows a namespace whose names
    // were cut, so a name of its own may be shadowed by one that is missing.
    expect(names).toContain("x")
    expect(names).toContain("b.y")
    expect(names).not.toContain("y")
  })

  it("reaches a source with no namespaces by the bare name", () => {
    const flat = snapshot(
      [namespace(null, [relation("things", loaded(["id", "int8"]))])],
      null
    )
    expect(applied("SELECT * FROM th|", flat)).toContainEqual({
      label: "things",
      apply: "things",
    })
  })
})

describe("after a qualifier", () => {
  it("offers the relations of the namespace, in their insertion form", () => {
    const options = applied("SELECT * FROM public.|")
    expect(options).toContainEqual({ label: "users", apply: "users" })
    expect(options).toContainEqual({ label: "order", apply: '"order"' })
    expect(options).toContainEqual({ label: HOSTILE, apply: HOSTILE_INSERT })
    expect(options).toContainEqual({
      label: "Order Lines",
      apply: '"Order Lines"',
    })
    expect(options.map((option) => option.label)).not.toContain("old_id")
  })

  it("compares an unquoted qualifier without case, a quoted one exactly", () => {
    expect(applied("SELECT * FROM PUBLIC.us|").length).toBeGreaterThan(0)
    expect(complete('SELECT * FROM "PUBLIC".us|')).toBeNull()
    expect(applied('SELECT * FROM "public".us|').length).toBeGreaterThan(0)
  })

  it("offers the columns of a relation, with the type beside them", () => {
    const result = complete("SELECT public.users.| FROM public.users")
    expect(result?.options.map((o) => [o.label, o.apply, o.detail])).toEqual([
      ["id", "id", "int8"],
      ["name", "name", "text"],
      ["Full Name", '"Full Name"', "text"],
    ])
  })

  it("reads a quoted qualifier, a doubled quote being one", () => {
    expect(applied('SELECT "a""b".|')).toEqual([{ label: "x", apply: "x" }])
    expect(applied('SELECT "Order Lines".|')).toEqual([
      { label: "qty", apply: "qty" },
    ])
  })

  it("offers nothing for a relation whose columns were never read", () => {
    expect(complete("SELECT public.order.|")).toBeNull()
  })

  it("offers nothing for a name the snapshot does not hold", () => {
    expect(complete("SELECT nowhere.|")).toBeNull()
    expect(complete("SELECT a.b.c.|")).toBeNull()
  })
})

describe("the statement's relations and aliases", () => {
  it("offers the columns behind an alias", () => {
    const labels = applied("SELECT u.| FROM users u").map((o) => o.label)
    expect(labels).toEqual(["id", "name", "Full Name"])
  })

  it("follows AS and a qualified relation", () => {
    expect(
      applied("SELECT x.| FROM archive.users AS x").map((o) => o.label)
    ).toEqual(["old_id"])
  })

  it("follows a join and a list of relations", () => {
    const joined = "SELECT * FROM users u JOIN orders o ON o.|"
    expect(applied(joined).map((o) => o.label)).toEqual(["id", "user_id"])
    const listed = "SELECT u.|, o.id FROM users u, orders o"
    expect(applied(listed).map((o) => o.label)).toEqual([
      "id",
      "name",
      "Full Name",
    ])
  })

  it("keeps an alias from reaching the wrong statement", () => {
    const two = "SELECT 1 FROM users u; SELECT u.| FROM orders o"
    expect(complete(two)).toBeNull()
  })

  it("offers the columns of the relations named, once each", () => {
    const result = complete("SELECT na| FROM users u JOIN orders o ON 1 = 1")
    const columns = (result?.options ?? []).filter(
      (option) => option.type === "property"
    )
    expect(columns.map((option) => option.label)).toEqual(
      expect.arrayContaining(["name", "user_id"])
    )
    // `id` is in both relations and offered once.
    expect(columns.filter((option) => option.label === "id")).toHaveLength(1)
    expect(columns.find((option) => option.label === "name")?.detail).toBe(
      "users · text"
    )
  })

  it("compares the relation named in the statement without case", () => {
    expect(applied("SELECT u.| FROM USERS u").map((o) => o.label)).toEqual([
      "id",
      "name",
      "Full Name",
    ])
  })

  it("does not follow a subquery: what it cannot read yields no column", () => {
    expect(complete("SELECT s.| FROM (SELECT id FROM users) s")).toBeNull()
  })

  it("does not take the names of a select list for relations", () => {
    const result = complete("SELECT users, orders| FROM nowhere")
    const columns = (result?.options ?? []).filter(
      (option) => option.type === "property"
    )
    expect(columns).toEqual([])
  })

  it("names no relation in a statement holding a name with a quote", () => {
    // The parser splits `"a""b"` in two: reading it could name the wrong table.
    expect(complete('SELECT x.| FROM "a""b" x')).toBeNull()
    const result = complete('SELECT na| FROM "a""b" x, users u')
    const columns = (result?.options ?? []).filter(
      (option) => option.type === "property"
    )
    expect(columns).toEqual([])
  })

  it("offers the columns of an INSERT target and an UPDATE target", () => {
    expect(
      applied("INSERT INTO users (na|) VALUES (1)").map((o) => o.label)
    ).toContain("name")
    expect(applied("UPDATE users SET na| = 1").map((o) => o.label)).toContain(
      "name"
    )
  })
})

describe("inside a quoted identifier", () => {
  it("replaces the quote and the name by the insertion form", () => {
    const text = 'SELECT * FROM public."Ord|"'
    const result = complete(text)
    const lines = text.indexOf("|")
    // From the opening quote, through the closing one that follows.
    expect(result?.from).toBe(text.indexOf('"Ord'))
    expect(result?.to).toBe(lines + 1)
    const found = result?.options.find((o) => o.displayLabel === "Order Lines")
    expect(found?.apply).toBe('"Order Lines"')
    // The label starts with the quote typed, so that it matches what was typed.
    expect(found?.label).toBe('"Order Lines"')
  })

  it("inserts a hostile name once doubled, never the label", () => {
    const result = complete('SELECT * FROM public."user|')
    const found = result?.options.find((o) => o.displayLabel === HOSTILE)
    expect(found?.apply).toBe(HOSTILE_INSERT)
    expect(found?.label).toBe(`"${HOSTILE}"`)
  })

  it("quotes with backticks for MySQL", () => {
    const mysql = snapshot(
      [namespace("shop", [relation("order items", loaded(["id", "int8"]))])],
      ["shop"]
    )
    const result = complete("SELECT * FROM shop.`ord|", mysql, MySQL)
    const found = result?.options.find((o) => o.displayLabel === "order items")
    expect(found?.label).toBe("`order items`")
  })
})

describe("what the snapshot flags", () => {
  it("shows a stale relation as such, and keeps suggesting it", () => {
    const stale = snapshot([
      namespace("public", [
        relation("gone", { state: "notLoaded" }, { stale: true }),
      ]),
    ])
    const result = complete("SELECT * FROM go|", stale)
    expect(result?.options.find((o) => o.label === "gone")?.detail).toBe(
      "table · stale"
    )
  })

  it("names the kind of a relation", () => {
    const views = snapshot([
      namespace("public", [
        relation(
          "totals",
          { state: "notLoaded" },
          { kind: "materialized_view" }
        ),
      ]),
    ])
    const result = complete("SELECT * FROM to|", views, SQLite)
    expect(result?.options.find((o) => o.label === "totals")?.detail).toBe(
      "materialized view"
    )
  })
})

/** The names the editor asks the host to read for, at the `|` of `text`. */
function wanted(text: string, dialect: Dialect = PostgreSQL) {
  const pos = text.indexOf("|")
  const doc = text.replace("|", "")
  const state = EditorState.create({ doc, extensions: [sql({ dialect })] })
  return wantedNames(state, pos)
}

const bare = (text: string) => ({ text, quoted: false })
const quoted = (text: string) => ({ text, quoted: true })

describe("what the statement asks of the cache", () => {
  it("names the relations of the statement, wherever the cursor is in it", () => {
    expect(
      wanted(
        "SELECT | FROM users u, archive.users a JOIN public.orders o ON o.user_id = u.id"
      )
    ).toEqual([
      [bare("users")],
      [bare("archive"), bare("users")],
      [bare("public"), bare("orders")],
    ])
  })

  it("keeps a quoted name quoted and unquotes its text", () => {
    expect(wanted('SELECT | FROM "Order Lines" ol')).toEqual([
      [quoted("Order Lines")],
    ])
  })

  it("reads the statement from its very start, as a console opens with the caret at 0", () => {
    expect(wanted("|SELECT * FROM users u")).toEqual([[bare("users")]])
  })

  it("reads only the statement around the cursor", () => {
    const text = "SELECT * FROM alpha; SELECT | FROM beta; SELECT * FROM gamma"
    expect(wanted(text)).toEqual([[bare("beta")]])
  })

  it("adds the qualifier being typed, with the part still unfinished", () => {
    expect(wanted("SELECT * FROM public.us|")).toEqual([
      [bare("public"), bare("us")],
    ])
    // The qualifier alone is also read as a relation of the statement, as
    // `orders.` is after `FROM orders`: the host finds none called `archive`
    // and reads nothing for it.
    expect(wanted("SELECT * FROM archive.|")).toEqual([
      [bare("archive")],
      [bare("archive"), bare("")],
    ])
    expect(wanted('SELECT * FROM "Sales"."Or|')).toEqual([
      [quoted("Sales"), quoted("Or")],
    ])
  })

  it("asks nothing for a name typed without a qualifier", () => {
    expect(wanted("SELECT * FROM us|")).toEqual([[bare("us")]])
    expect(wanted("SELECT * FROM |")).toEqual([])
  })

  it("lists a name once", () => {
    expect(wanted("SELECT | FROM users JOIN users u2 ON true")).toEqual([
      [bare("users")],
    ])
  })

  it("asks nothing in a string, a comment or an empty text", () => {
    expect(wanted("SELECT 'FROM users|'")).toEqual([])
    expect(wanted("-- FROM users|")).toEqual([])
    expect(wanted("|")).toEqual([])
  })

  it("does not read a statement whose name holds a quote the parser splits", () => {
    expect(wanted('SELECT | FROM "a""b"')).toEqual([])
  })

  it("does not follow a subquery", () => {
    expect(wanted("SELECT | FROM (SELECT 1) t")).toEqual([])
  })
})
