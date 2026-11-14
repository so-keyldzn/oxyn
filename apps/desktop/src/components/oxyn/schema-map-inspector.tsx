import { Badge } from "@/components/ui/badge"
import { Button } from "@/components/ui/button"
import {
  Card,
  CardHeader,
  CardTitle,
  CardDescription,
  CardContent,
  CardFooter,
} from "@/components/ui/card"
import { ScrollArea } from "@/components/ui/scroll-area"
import type { MapObject, SchemaMapProjection } from "@/lib/ipc/schema-map"
import type { OpenTarget } from "./catalog-tree"

/** Inspection uses the projected metadata only. Actions are explicit owner-bound gestures. */
export function SchemaMapInspector({
  object,
  projection,
  busy,
  definitionSupported,
  onOpen,
  onExpand,
  onInfer,
}: {
  object: MapObject
  projection: SchemaMapProjection
  busy: boolean
  definitionSupported: boolean
  onOpen: (object: MapObject, target: OpenTarget) => void
  onExpand: (object: MapObject) => void
  onInfer?: (object: MapObject) => void
}) {
  const related = projection.links.filter(
    (link) =>
      link.kind === "foreignKey" &&
      (link.from === object.key || link.to === object.key)
  )
  return (
    <Card className="min-h-0 min-w-0 gap-3 py-3">
      <CardHeader className="px-3">
        <CardTitle className="truncate">
          <bdi>{object.address.relation}</bdi>
        </CardTitle>
        <CardDescription className="wrap-anywhere">
          {[object.address.catalog, object.address.namespace]
            .filter((part) => part !== null)
            .join(" / ") || "Source root"}
        </CardDescription>
        <div className="flex flex-wrap gap-1">
          <Badge variant="secondary">{object.kind.replaceAll("_", " ")}</Badge>
          <Badge variant="outline">Fields: {object.fieldsState}</Badge>
          <Badge variant="outline">
            Keys: {object.keysSupported ? object.keysState : "unavailable"}
          </Badge>
        </div>
      </CardHeader>
      <CardContent className="min-h-0 flex-1 overflow-hidden px-3">
        <ScrollArea className="h-full max-h-96 min-h-32 [&>[data-slot=scroll-area-viewport]]:max-h-96">
          <div className="flex flex-col gap-3 pr-2">
            <p className="text-xs text-muted-foreground">
              Selection reads no rows. Reported metadata can be partial;
              incoming links cover loaded outgoing keys only.
            </p>
            {object.fieldsState === "unread" ? (
              <p className="text-sm">Field structure has not been read.</p>
            ) : object.fields.length === 0 ? (
              <p className="text-sm">No fields reported in the last read.</p>
            ) : (
              <ul aria-label="Projected fields" className="flex flex-col gap-2">
                {object.fields.map((field) => (
                  <li
                    key={field.key}
                    className="flex flex-col gap-1 text-xs wrap-anywhere"
                  >
                    <span>
                      <bdi>{field.path.join(" › ")}</bdi> · {field.rawType}
                      {field.typeShortened ? "… (type shortened)" : ""}
                    </span>
                    <div className="flex flex-wrap gap-1">
                      <Badge variant="outline">
                        {field.origin === "sampled"
                          ? "Sample-derived"
                          : field.origin === "previewLayout"
                            ? "Preview layout"
                            : "Reported"}
                      </Badge>
                      {field.primaryKey && (
                        <Badge variant="secondary">Primary key</Badge>
                      )}
                      {field.parent && (
                        <Badge variant="outline">Nested member</Badge>
                      )}
                    </div>
                  </li>
                ))}
              </ul>
            )}
            {object.omittedFields > 0 && (
              <p className="text-sm">
                {object.omittedFields} more fields omitted. Open Structure for
                the retained full description.
              </p>
            )}
            {object.samplingRequired && (
              <p className="text-sm">
                Structure requires a separate, bounded document sample. No
                values are included in this map.
              </p>
            )}
            {related.length > 0 ? (
              <ul
                aria-label="Reported links"
                className="flex flex-col gap-3 text-xs"
              >
                {related.map((link) => (
                  <li
                    key={link.key}
                    className="flex flex-col gap-1 wrap-anywhere"
                  >
                    <span>
                      {link.name || "Unnamed foreign key"}: (
                      {link.fields.join(", ")}) →{" "}
                      {[
                        link.target.catalog,
                        link.target.namespace,
                        link.target.relation,
                      ]
                        .filter((part) => part !== null)
                        .join(".")}{" "}
                      ({link.referencedFields.join(", ")})
                    </span>
                    <span>
                      {link.freshness === "stale" ? "Retained stale key. " : ""}
                      {!link.targetLoaded ? "Target not loaded. " : ""}
                      {!link.wellFormed
                        ? "Incomplete reported key; members are not guessed. "
                        : ""}
                      {link.sourceUnique === null || link.targetUnique === null
                        ? "Cardinality is unknown where uniqueness was not reported."
                        : "Reported keys prove at most one matching row on each side."}
                    </span>
                  </li>
                ))}
              </ul>
            ) : (
              <p className="text-xs text-muted-foreground">
                No foreign key appears among the loaded objects. This does not
                prove that none exist.
              </p>
            )}
          </div>
        </ScrollArea>
      </CardContent>
      <CardFooter className="flex flex-wrap gap-2 px-3">
        <Button
          variant="outline"
          size="sm"
          disabled={busy}
          onClick={() => onOpen(object, "structure")}
        >
          Open Structure
        </Button>
        {object.holdsRecords && (
          <Button
            variant="outline"
            size="sm"
            disabled={busy}
            onClick={() => onOpen(object, "data")}
          >
            Open Data
          </Button>
        )}
        {definitionSupported && (
          <Button
            variant="outline"
            size="sm"
            disabled={busy}
            onClick={() => onOpen(object, "definition")}
          >
            Open definition
          </Button>
        )}
        <Button
          variant="outline"
          size="sm"
          disabled={busy}
          onClick={() => onExpand(object)}
        >
          Expand neighbours
        </Button>
        {object.samplingRequired && (
          <Button
            size="sm"
            disabled={busy || !projection.sampleLimits || !onInfer}
            onClick={() => onInfer?.(object)}
          >
            Infer fields…
          </Button>
        )}
        {object.samplingRequired && !projection.sampleLimits && (
          <p className="text-xs text-muted-foreground">
            This source has not declared sampling bounds.
          </p>
        )}
      </CardFooter>
    </Card>
  )
}
