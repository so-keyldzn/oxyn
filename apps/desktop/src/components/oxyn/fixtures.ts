// Story and test data. Nothing here is shown in the application: the
// workspace only ever draws what the backend returned (docs/UX-SPEC.md).

import type { FetchPage } from "./result-grid"
import type {
  CatalogNode,
  Cell,
  DriverChoice,
  RelationDetail,
  ResultColumn,
  SavedConnection,
} from "@/lib/ipc/types"

export const postgresDriver: DriverChoice = {
  id: "postgres",
  displayName: "PostgreSQL",
  family: "relational",
  defaultPort: 5432,
  fields: [
    {
      key: "host",
      label: "Host",
      kind: { type: "text" },
      required: true,
      secret: false,
      default: "localhost",
      help: null,
    },
    {
      key: "port",
      label: "Port",
      kind: { type: "number" },
      required: false,
      secret: false,
      default: "5432",
      help: null,
    },
    {
      key: "database",
      label: "Database",
      kind: { type: "text" },
      required: true,
      secret: false,
      default: null,
      help: null,
    },
    {
      key: "user",
      label: "User",
      kind: { type: "text" },
      required: true,
      secret: false,
      default: null,
      help: null,
    },
    {
      key: "password",
      label: "Password",
      kind: { type: "password" },
      required: false,
      secret: true,
      default: null,
      help: "Stored in the system keyring.",
    },
    {
      key: "sslmode",
      label: "TLS mode",
      kind: {
        type: "choice",
        options: ["disable", "prefer", "require", "verify-full"],
      },
      required: false,
      secret: false,
      default: "prefer",
      help: null,
    },
  ],
}

export const sqliteDriver: DriverChoice = {
  id: "sqlite",
  displayName: "SQLite",
  family: "embedded",
  defaultPort: null,
  fields: [
    {
      key: "path",
      label: "Database file",
      kind: { type: "path" },
      required: true,
      secret: false,
      default: null,
      help: null,
    },
    {
      key: "read_only",
      label: "Open read only",
      kind: { type: "bool" },
      required: false,
      secret: false,
      default: "false",
      help: null,
    },
  ],
}

export const savedConnections: Array<SavedConnection> = [
  {
    id: "018f0000-0000-7000-8000-000000000001",
    name: "billing",
    driver: "postgres",
    environment: "production",
    readOnly: false,
    privacyTier: "metadata",
  },
  {
    id: "018f0000-0000-7000-8000-000000000002",
    name: "billing replica",
    driver: "postgres",
    environment: "staging",
    readOnly: true,
    privacyTier: "metadata",
  },
  {
    id: "018f0000-0000-7000-8000-000000000003",
    name: "scratch.sqlite",
    driver: "sqlite",
    environment: "local",
    readOnly: false,
    privacyTier: "metadata",
  },
]

const relation = (
  namespace: string,
  name: string,
  kind = "table"
): CatalogNode => ({
  address: { catalog: "billing", namespace, relation: name },
  name,
  kind,
  holdsRecords: kind !== "function",
  system: false,
  comment: null,
  loaded: true,
  stale: false,
  children: [],
})

export const catalog: Array<CatalogNode> = [
  {
    address: { catalog: "billing", namespace: "public", relation: null },
    name: "public",
    kind: "namespace",
    holdsRecords: false,
    system: false,
    comment: null,
    loaded: true,
    stale: false,
    children: [
      relation("public", "customers"),
      relation("public", "invoices"),
      relation("public", "invoice_lines"),
      relation("public", "active_customers", "view"),
      relation("public", 'users"; DROP TABLE audit; --'),
    ],
  },
  {
    address: { catalog: "billing", namespace: "reporting", relation: null },
    name: "reporting",
    kind: "namespace",
    holdsRecords: false,
    system: false,
    comment: null,
    loaded: false,
    stale: false,
    children: [],
  },
  {
    address: { catalog: "billing", namespace: "pg_catalog", relation: null },
    name: "pg_catalog",
    kind: "namespace",
    holdsRecords: false,
    system: true,
    comment: null,
    loaded: false,
    stale: false,
    children: [],
  },
]

export const invoiceColumns: Array<ResultColumn> = [
  { name: "id", dataType: "Int64", nullable: false },
  { name: "customer", dataType: "Utf8", nullable: false },
  { name: "amount", dataType: "Decimal128(12, 2)", nullable: false },
  {
    name: "issued_at",
    dataType: 'Timestamp(Microsecond, Some("UTC"))',
    nullable: false,
  },
  {
    name: "paid_at",
    dataType: 'Timestamp(Microsecond, Some("UTC"))',
    nullable: true,
  },
  { name: "notes", dataType: "Utf8", nullable: true },
  { name: "payload", dataType: "Binary", nullable: true },
]

const customers = [
  "Acme SA",
  "Globex",
  "Initech",
  "Umbrella",
  "Hooli",
  "Stark Industries",
]

function cellFor(row: number, column: number): Cell {
  switch (column) {
    case 0:
      return String(row + 1)
    case 1:
      return customers[row % customers.length] ?? null
    case 2:
      return (((row * 7919) % 100000) / 100).toFixed(2)
    case 3:
      return new Date(Date.UTC(2026, 0, 1) + row * 60_000).toISOString()
    case 4:
      return row % 3 === 0
        ? null
        : new Date(Date.UTC(2026, 0, 2) + row * 60_000).toISOString()
    case 5:
      return row % 50 === 0
        ? { text: "Long note ".repeat(40), fullBytes: 48_213 }
        : row % 4 === 0
          ? "NULL"
          : null
    default:
      return row % 97 === 0
        ? { unrenderable: "unsupported type: Dictionary" }
        : "\\x89504e47"
  }
}

/**
 * A result of `rows` rows served page by page, with a small simulated delay.
 *
 * Answers the shape the backend answers with — `type: "page"` included: a
 * fixture that dropped the tag let the grid's « expired » test pass on every
 * page without a single story noticing.
 */
export function syntheticPages(rows: number, delayMs = 60): FetchPage {
  return (offset: number, limit: number) =>
    new Promise((resolve) => {
      setTimeout(() => {
        const end = Math.min(rows, offset + limit)
        const page: Array<Array<Cell>> = []
        for (let row = offset; row < end; row++) {
          page.push(invoiceColumns.map((_, column) => cellFor(row, column)))
        }
        resolve({
          type: "page",
          offset,
          rows: page,
          totalRows: rows,
          complete: true,
        })
      }, delayMs)
    })
}

export const invoicesDetail: RelationDetail = {
  name: "invoices",
  kind: "table",
  comment: "One row per issued invoice.",
  estimatedRows: 1_284_512,
  sizeBytes: 412_000_000,
  fields: [
    {
      name: "id",
      position: 1,
      logicalType: "int64",
      rawType: "bigint",
      nullable: false,
      default: "nextval('invoices_id_seq')",
      comment: null,
      primaryKey: true,
    },
    {
      name: "customer_id",
      position: 2,
      logicalType: "int64",
      rawType: "bigint",
      nullable: false,
      default: null,
      comment: "References customers.id",
      primaryKey: false,
    },
    {
      name: "amount",
      position: 3,
      logicalType: "decimal",
      rawType: "numeric(12,2)",
      nullable: false,
      default: null,
      comment: null,
      primaryKey: false,
    },
    {
      name: "issued_at",
      position: 4,
      logicalType: "timestamp",
      rawType: "timestamptz",
      nullable: false,
      default: "now()",
      comment: null,
      primaryKey: false,
    },
    {
      name: "paid_at",
      position: 5,
      logicalType: "timestamp",
      rawType: "timestamptz",
      nullable: true,
      default: null,
      comment: null,
      primaryKey: false,
    },
  ],
}

/**
 * The catalogue docs/VISION.md aims at, as a build registering all of it
 * would list it: the start screen must stay a launcher at that size. Fields
 * are borrowed from the two real drivers, by how each one is reached.
 */
export const catalogueDrivers: Array<DriverChoice> = [
  postgresDriver,
  ...(
    [
      ["mysql", "MySQL", "relational", 3306],
      ["mariadb", "MariaDB", "relational", 3306],
      ["sqlserver", "SQL Server", "relational", 1433],
      ["oracle", "Oracle", "relational", 1521],
      ["duckdb", "DuckDB", "analytical", null],
      ["clickhouse", "ClickHouse", "analytical", 8123],
      ["snowflake", "Snowflake", "analytical", null],
      ["bigquery", "BigQuery", "analytical", null],
      ["mongodb", "MongoDB", "document", 27017],
      ["redis", "Redis", "key-value", 6379],
      ["cassandra", "Cassandra", "wide-column", 9042],
      ["dynamodb", "DynamoDB", "key-value", null],
      ["couchbase", "Couchbase", "document", 8091],
      ["milvus", "Milvus", "vector", 19530],
      ["weaviate", "Weaviate", "vector", 8080],
      ["pinecone", "Pinecone", "vector", null],
      ["qdrant", "Qdrant", "vector", 6333],
      ["chromadb", "ChromaDB", "vector", 8000],
      ["neo4j", "Neo4j", "graph", 7687],
      ["memgraph", "Memgraph", "graph", 7687],
      ["elasticsearch", "Elasticsearch", "search", 9200],
      ["opensearch", "OpenSearch", "search", 9200],
    ] as const
  ).map(([id, displayName, family, defaultPort]): DriverChoice =>
    id === "duckdb"
      ? { ...sqliteDriver, id, displayName, family }
      : { ...postgresDriver, id, displayName, family, defaultPort }
  ),
  sqliteDriver,
]
