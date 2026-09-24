// Tables and keys for the diagram stories. No connection identifier anywhere:
// a story is published with Storybook (I-03).

import type { ErdColumn, ErdLink, ErdTable } from "./erd-model"

const column = (
  name: string,
  type: string,
  mark: "pk" | "fk" | null = null
): ErdColumn => ({
  name,
  type,
  primaryKey: mark === "pk",
  foreignKey: mark === "fk",
})

export const erdTable = (
  name: string,
  columns: Array<ErdColumn>,
  requested = true,
  namespace: string | null = "public"
): ErdTable => ({
  key: JSON.stringify([null, namespace, name]),
  address: { catalog: null, namespace, relation: name },
  name,
  namespace,
  columns,
  requested,
})

export const erdLink = (
  from: ErdTable,
  to: ErdTable,
  name: string,
  fields: Array<string>
): ErdLink => ({
  key: JSON.stringify([from.key, name]),
  from: from.key,
  to: to.key,
  name,
  fields,
  referencedFields: ["id"],
})

const customers = erdTable("customers", [
  column("id", "bigint", "pk"),
  column("email", "text"),
  column("created_at", "timestamptz"),
])
const orders = erdTable("orders", [
  column("id", "bigint", "pk"),
  column("customer_id", "bigint", "fk"),
  column("status", "order_status"),
  column("total", "numeric(12,2)"),
  column("placed_at", "timestamptz"),
])
const items = erdTable(
  "order_items",
  [
    column("id", "bigint", "pk"),
    column("order_id", "bigint", "fk"),
    column("product_id", "bigint", "fk"),
    column("quantity", "integer"),
  ],
  false
)
const products = erdTable(
  "products",
  [
    column("id", "bigint", "pk"),
    column("sku", "text"),
    column("name", "text"),
    ...Array.from({ length: 14 }, (_, i) =>
      column(`attribute_${i + 1}`, "text")
    ),
  ],
  false
)
/** A name the catalog holds, and that would be SQL if concatenated (I-10). */
const hostile = erdTable(
  '"users"; DROP TABLE audit; --',
  [column("id", "uuid", "pk"), column("order_id", "bigint", "fk")],
  false
)

export const shopTables = [customers, orders, items, products, hostile]
export const shopLinks = [
  erdLink(orders, customers, "orders_customer_fk", ["customer_id"]),
  erdLink(items, orders, "items_order_fk", ["order_id"]),
  erdLink(items, products, "items_product_fk", ["product_id"]),
  erdLink(hostile, orders, "hostile_order_fk", ["order_id"]),
]

/** A hub and its spokes, cut at the bound. */
export function hubAndSpokes(spokes: number) {
  const hub = erdTable("events", [column("id", "bigint", "pk")])
  const tables = [hub]
  const links: Array<ErdLink> = []
  for (let index = 0; index < spokes; index += 1) {
    const spoke = erdTable(
      `event_detail_${index + 1}`,
      [column("id", "bigint", "pk"), column("event_id", "bigint", "fk")],
      false
    )
    tables.push(spoke)
    links.push(
      erdLink(spoke, hub, `detail_${index + 1}_event_fk`, ["event_id"])
    )
  }
  return { tables, links }
}
