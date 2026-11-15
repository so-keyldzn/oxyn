import type { ErdLink, ErdTable } from "@/components/oxyn/erd-model"
import type { SchemaMapProjection } from "@/lib/ipc/schema-map"

/** One renderer adapter for host-projected maps, including assistant diagrams. */
export function erdProjection(projection: SchemaMapProjection): {
  tables: Array<ErdTable>
  links: Array<ErdLink>
} {
  const links = projection.links.filter(
    (link) => link.kind === "foreignKey" && link.targetLoaded
  )
  return {
    tables: projection.objects.map((object) => ({
      key: object.key,
      address: object.address,
      name: object.address.relation ?? "",
      namespace: object.address.namespace,
      catalog: object.address.catalog,
      kind: object.kind,
      fieldsState: object.fieldsState,
      columnsKnown: object.fieldsState !== "unread",
      requested: true,
      columns: object.fields.map((field) => ({
        key: field.key,
        name: field.path.join(" › "),
        type: field.rawType,
        primaryKey: field.primaryKey,
        foreignKey:
          field.parent === null &&
          links.some(
            (link) =>
              link.from === object.key && link.fields.includes(field.name)
          ),
        origin: field.origin,
        depth: field.path.length - 1,
      })),
    })),
    links: links.map((link) => ({
      key: link.key,
      from: link.from,
      to: link.to,
      name: link.name,
      fields: link.fields,
      referencedFields: link.referencedFields,
      stale: link.freshness === "stale",
    })),
  }
}
