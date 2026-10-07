import type { CatalogNode, VirtualTableMark } from "@/lib/ipc/types"

/**
 * An SQLite database written by a tool that loads `sqlite-vec`: one virtual
 * table on a module this connection lacks, one on a built-in module, and their
 * shadow tables.
 */
const table = (
  name: string,
  marks: {
    virtualTable?: VirtualTableMark
    shadowOf?: string
  } = {}
): CatalogNode => ({
  address: { catalog: null, namespace: "main", relation: name },
  name,
  kind: "table",
  holdsRecords: true,
  system: false,
  comment: null,
  loaded: true,
  stale: false,
  children: [],
  virtualTable: marks.virtualTable ?? null,
  shadowOf: marks.shadowOf ?? null,
})

const VEC_SHADOWS = [
  "chunks_vec_chunks",
  "chunks_vec_info",
  "chunks_vec_rowids",
  "chunks_vec_vector_chunks00",
]

export const missingModuleTable = table("chunks_vec", {
  virtualTable: { module: "vec0", available: false, shadows: VEC_SHADOWS },
})

export const virtualCatalog: Array<CatalogNode> = [
  {
    address: { catalog: null, namespace: "main", relation: null },
    name: "main",
    kind: "namespace",
    holdsRecords: false,
    system: false,
    comment: null,
    loaded: true,
    stale: false,
    children: [
      table("chunks"),
      missingModuleTable,
      ...VEC_SHADOWS.map((name) => table(name, { shadowOf: "chunks_vec" })),
      table("docs", {
        virtualTable: {
          module: "fts5",
          available: true,
          shadows: ["docs_config", "docs_data"],
        },
      }),
      table("docs_config", { shadowOf: "docs" }),
      table("docs_data", { shadowOf: "docs" }),
    ],
    virtualTable: null,
    shadowOf: null,
  },
]

/** The engine's words, as the driver returns them. */
export const missingModuleMessage =
  "driver `sqlite` (permanent error): no such module: vec0"
