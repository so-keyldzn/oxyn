/**
 * The language a console writes to a session, read from its capabilities.
 *
 * The same rule as `oxyn_query::console_language`: SQL when the session
 * declares it, otherwise its one other language, and `null` when it declares
 * none or several besides SQL. The names are those of `QueryLanguage::as_str`.
 */

/** Each language flag a session can declare, and the language it names. */
const LANGUAGE_FLAGS = {
  CYPHER: "cypher",
  GREMLIN: "gremlin",
  MONGO_QUERY: "mongo",
  REDIS_COMMAND: "redis",
  SEARCH_DSL: "search-dsl",
  CQL: "cql",
  PARTIQL: "partiql",
  INFLUXQL: "influxql",
  FLUX: "flux",
} as const

export type QueryLanguageName =
  "sql" | (typeof LANGUAGE_FLAGS)[keyof typeof LANGUAGE_FLAGS]

/** What a sentence calls each language. */
export const QUERY_LANGUAGE_LABELS: Record<QueryLanguageName, string> = {
  sql: "SQL",
  cypher: "Cypher",
  gremlin: "Gremlin",
  mongo: "MongoDB queries",
  redis: "Redis commands",
  "search-dsl": "the Elasticsearch Query DSL",
  cql: "CQL",
  partiql: "PartiQL",
  influxql: "InfluxQL",
  flux: "Flux",
}

export function queryLanguageOf(
  capabilities: ReadonlyArray<string>
): QueryLanguageName | null {
  if (capabilities.includes("SQL")) return "sql"
  const [only, ...others] = Object.entries(LANGUAGE_FLAGS)
    .filter(([flag]) => capabilities.includes(flag))
    .map(([, language]) => language)
  return only !== undefined && others.length === 0 ? only : null
}
