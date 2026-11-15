import type { SchemaMapProjection } from "@/lib/ipc/schema-map"

/** Every semantic link is readable even when its kind has no canvas edge renderer. */
export function SchemaMapLinkList({
  projection,
}: {
  projection: SchemaMapProjection
}) {
  const names = new Map<string, string>()
  const label = (address: {
    catalog: string | null
    namespace: string | null
    relation: string | null
  }) =>
    [address.catalog, address.namespace, address.relation]
      .filter((part) => part !== null)
      .join(" / ") || "Source root"
  for (const container of projection.containers)
    names.set(container.key, label(container.address))
  for (const object of projection.objects) {
    names.set(object.key, label(object.address))
    for (const field of object.fields)
      names.set(
        field.key,
        `${label(object.address)} / ${field.path.join(" › ")}`
      )
  }
  return (
    <div className="flex flex-col gap-2 p-2 text-xs">
      <h3 className="font-semibold">Reported relationships and structure</h3>
      {projection.links.length === 0 ? (
        <p>No link was reported in the retained metadata.</p>
      ) : (
        <ul aria-label="All mapped links" className="flex flex-col gap-2">
          {projection.links.map((link) => (
            <li key={link.key} className="wrap-anywhere">
              {link.kind === "foreignKey"
                ? "Foreign key"
                : link.kind === "containment"
                  ? "Field nesting"
                  : link.kind === "membership"
                    ? "Native membership"
                    : "Technical dependency"}
              : {names.get(link.from) ?? "Unloaded source"} (
              {link.fields.join(", ")}) →{" "}
              {names.get(link.to) ?? label(link.target)} (
              {link.referencedFields.join(", ")}). {link.freshness}.{" "}
              {!link.targetLoaded ? "Endpoint not loaded." : ""}{" "}
              {!link.wellFormed
                ? "Incomplete key; members are not guessed."
                : ""}
            </li>
          ))}
        </ul>
      )}
    </div>
  )
}
