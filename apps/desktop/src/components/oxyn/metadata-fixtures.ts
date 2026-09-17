// Story and test data for the object view. Nothing here is shown in the
// application: the workspace only ever draws what the backend returned.

import { invoicesDetail } from "./fixtures"
import type {
  ConstraintRow,
  DefinitionView,
  ForeignKeyRow,
  IncomingKeyRow,
  IndexRow,
  RelationFacets,
} from "@/lib/ipc/metadata"
import type {
  CatalogAddress,
  CatalogNode,
  RelationDetail,
} from "@/lib/ipc/types"

export const HOSTILE = 'users"; DROP TABLE audit; --'

export const invoicesAddress: CatalogAddress = {
  catalog: "billing",
  namespace: "public",
  relation: "invoices",
}

export const hostileAddress: CatalogAddress = {
  catalog: "billing",
  namespace: "public",
  relation: HOSTILE,
}

export const fetched = {
  state: "fetched",
  fetchedAt: "2026-09-15T09:30:00+00:00",
} as const

export const invoiceIndexes: Array<IndexRow> = [
  {
    name: "invoices_pkey",
    fields: ["id"],
    unique: true,
    method: "btree",
    predicate: null,
  },
  {
    name: "invoices_unpaid_idx",
    fields: ["customer_id", "issued_at"],
    unique: false,
    method: "btree",
    predicate: "paid_at IS NULL",
  },
]

export const invoiceConstraints: Array<ConstraintRow> = [
  {
    name: "invoices_pkey",
    kind: "primary_key",
    fields: ["id"],
    expression: "PRIMARY KEY (id)",
    validated: true,
  },
  {
    name: "invoices_amount_positive",
    kind: "check",
    fields: ["amount"],
    expression: "CHECK (amount > 0)",
    validated: false,
  },
  {
    name: "",
    kind: "not_null",
    fields: [],
    expression: null,
    validated: null,
  },
]

export const customerKey: ForeignKeyRow = {
  name: "invoices_customer_fkey",
  fields: ["customer_id"],
  references: {
    catalog: "billing",
    namespace: "public",
    relation: "customers",
  },
  referencedFields: ["id"],
  onDelete: "cascade",
  cascades: true,
  wellFormed: true,
}

export const incomingKeys: Array<IncomingKeyRow> = [
  {
    source: {
      catalog: "billing",
      namespace: "public",
      relation: "invoice_lines",
    },
    key: {
      name: "invoice_lines_invoice_fkey",
      fields: ["invoice_id"],
      references: invoicesAddress,
      referencedFields: ["id"],
      onDelete: "no_action",
      cascades: false,
      wellFormed: true,
    },
    sourceUnique: false,
  },
  {
    source: hostileAddress,
    key: {
      name: "",
      fields: ["last_invoice"],
      references: invoicesAddress,
      referencedFields: ["id"],
      onDelete: "set_null",
      cascades: true,
      wellFormed: true,
    },
    sourceUnique: null,
  },
]

export const invoicesDefinition: DefinitionView = {
  sql: `CREATE TABLE public.invoices (
  id bigint NOT NULL DEFAULT nextval('invoices_id_seq'),
  customer_id bigint NOT NULL,
  amount numeric(12,2) NOT NULL,
  issued_at timestamptz NOT NULL DEFAULT now(),
  paid_at timestamptz,
  CONSTRAINT invoices_pkey PRIMARY KEY (id),
  CONSTRAINT invoices_amount_positive CHECK (amount > 0)
);

CREATE INDEX invoices_unpaid_idx ON public.invoices (customer_id, issued_at)
  WHERE paid_at IS NULL;`,
  source: "reconstructed",
  notes: [
    "Privileges, data and dependent objects are not included.",
    "Rebuilt from pg_catalog; comments are listed separately.",
  ],
}

export const invoicesFacets: RelationFacets = {
  address: invoicesAddress,
  kind: "table",
  holdsRecords: true,
  qualifiedName: '"public"."invoices"',
  detail: { freshness: fetched, value: invoicesDetail },
  indexes: invoiceIndexes,
  foreignKeys: [customerKey],
  constraints: { freshness: fetched, value: invoiceConstraints },
  incomingKeys: { freshness: fetched, value: incomingKeys },
  definition: { freshness: fetched, value: invoicesDefinition },
  uniqueKey: true,
}

export const hostileFacets: RelationFacets = {
  ...invoicesFacets,
  address: hostileAddress,
  // Quoted by the backend, as `qualified_name` renders it for PostgreSQL.
  qualifiedName: '"public"."users""; DROP TABLE audit; --"',
  detail: {
    freshness: { state: "invalidated" },
    value: { ...invoicesDetail, name: HOSTILE, comment: null, sizeBytes: null },
  },
}

const catalogRelation = (
  namespace: string,
  name: string,
  kind = "table",
  comment: string | null = null
): CatalogNode => ({
  address: { catalog: "billing", namespace, relation: name },
  name,
  kind,
  holdsRecords: kind !== "function",
  system: false,
  comment,
  loaded: true,
  stale: false,
  children: [],
})

const catalogNamespace = (
  name: string,
  children: Array<CatalogNode>,
  overrides: Partial<CatalogNode> = {}
): CatalogNode => ({
  address: { catalog: "billing", namespace: name, relation: null },
  name,
  kind: "namespace",
  holdsRecords: false,
  system: false,
  comment: null,
  loaded: true,
  stale: false,
  children,
  ...overrides,
})

/** One schema of 5 000 relations: the tree renders the visible rows only. */
export const hugeCatalog: Array<CatalogNode> = [
  catalogNamespace(
    "events",
    Array.from({ length: 5000 }, (_, index) =>
      catalogRelation(
        "events",
        `event_${String(index).padStart(4, "0")}`,
        index % 7 === 0 ? "view" : "table"
      )
    )
  ),
]

/** Names a catalog legally holds: very long, hostile, right-to-left, mixed scripts. */
export const unusualCatalog: Array<CatalogNode> = [
  catalogNamespace("public", [
    catalogRelation(
      "public",
      "customer_lifetime_value_by_acquisition_channel_and_region_monthly_snapshot_v2",
      "table",
      "A long name is truncated in the tree and read in full in its title."
    ),
    catalogRelation("public", HOSTILE),
    catalogRelation("public", "العملاء", "table", "جدول العملاء"),
    catalogRelation("public", "הזמנות"),
    catalogRelation("public", "注文明細"),
    catalogRelation("public", "émissions_CO₂"),
    catalogRelation("public", "a.b", "view"),
  ]),
  catalogNamespace("مخطط", [catalogRelation("مخطط", "سجل")]),
  catalogNamespace("archive", [], { loaded: false }),
]

/** A relation of 1 200 columns: the structure list renders the visible ones. */
export const wideDetail: RelationDetail = {
  name: "sensor_readings_wide",
  kind: "table",
  comment: "One column per sensor.",
  estimatedRows: 48_000_000,
  sizeBytes: 91_000_000_000,
  fields: Array.from({ length: 1200 }, (_, index) => ({
    name: index === 0 ? "id" : `sensor_${String(index).padStart(4, "0")}`,
    position: index + 1,
    logicalType: index === 0 ? "int64" : "float64",
    rawType: index === 0 ? "bigint" : "double precision",
    nullable: index !== 0,
    default: null,
    comment: index % 100 === 0 ? `Calibrated batch ${index / 100}` : null,
    primaryKey: index === 0,
  })),
}

/** Column names and comments from other scripts, and a hostile name. */
export const unusualDetail: RelationDetail = {
  ...invoicesDetail,
  name: HOSTILE,
  comment: "تعليق باللغة العربية على الجدول",
  fields: [
    ...invoicesDetail.fields,
    {
      name: HOSTILE,
      position: invoicesDetail.fields.length + 1,
      logicalType: "string",
      rawType: "text",
      nullable: true,
      default: "'--'::text",
      comment: null,
      primaryKey: false,
    },
    {
      name: "اسم_العميل",
      position: invoicesDetail.fields.length + 2,
      logicalType: "string",
      rawType: "character varying(255)",
      nullable: false,
      default: null,
      comment: "الاسم الكامل",
      primaryKey: false,
    },
  ],
}
