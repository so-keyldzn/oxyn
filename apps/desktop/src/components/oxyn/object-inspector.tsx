import { HugeiconsIcon } from "@hugeicons/react"
import { Copy01Icon, InformationCircleIcon } from "@hugeicons/core-free-icons"

import { freshnessLabel } from "@/components/oxyn/facet-frame"
import { Badge } from "@/components/ui/badge"
import { Button } from "@/components/ui/button"
import {
  Empty,
  EmptyDescription,
  EmptyHeader,
  EmptyMedia,
  EmptyTitle,
} from "@/components/ui/empty"
import { Skeleton } from "@/components/ui/skeleton"
import type { RelationFacets } from "@/lib/ipc/metadata"

/** Bytes for a reader: a relation size, never a cell (cells format in Rust). */
export function sizeLabel(bytes: number) {
  const units = ["B", "KiB", "MiB", "GiB", "TiB"]
  let value = bytes
  let unit = 0
  while (value >= 1024 && unit < units.length - 1) {
    value /= 1024
    unit++
  }
  return `${unit === 0 ? value : value.toFixed(1)} ${units[unit]}`
}

/**
 * What the catalog knows about the selected object, for the side column.
 *
 * Nothing is counted: rows are the engine's estimate when it gives one, and
 * « not reported » otherwise — a `COUNT(*)` would scan the table. The
 * qualified name comes quoted from the backend and is copied as is.
 */
export function ObjectInspector({
  facets,
  loading,
  onCopyName,
}: {
  /** `null` when no object is selected. */
  facets: RelationFacets | null
  loading: boolean
  onCopyName: (qualifiedName: string) => void
}) {
  if (!facets) {
    return (
      <Empty className="h-full border-0">
        <EmptyHeader>
          <EmptyMedia variant="icon">
            <HugeiconsIcon icon={InformationCircleIcon} strokeWidth={2} />
          </EmptyMedia>
          <EmptyTitle>Object inspector</EmptyTitle>
          <EmptyDescription>
            Select a table or a view in the explorer to see what the catalog
            knows about it.
          </EmptyDescription>
        </EmptyHeader>
      </Empty>
    )
  }

  const detail = facets.detail.value
  const primaryKey = detail?.fields.filter((field) => field.primaryKey) ?? []

  return (
    <div className="flex h-full min-h-0 flex-col gap-3 overflow-auto p-3 text-sm">
      <div className="flex flex-col gap-1">
        <div className="flex items-center gap-2">
          <span
            dir="auto"
            title={facets.address.relation ?? undefined}
            className="min-w-0 truncate font-medium"
          >
            {facets.address.relation}
          </span>
          {facets.kind ? (
            <Badge variant="secondary">{facets.kind}</Badge>
          ) : null}
          <Badge variant="outline" className="ml-auto">
            {freshnessLabel(facets.detail.freshness)}
          </Badge>
        </div>
        <div className="flex min-w-0 items-center gap-1">
          <code
            data-selectable
            className="truncate font-mono text-xs text-muted-foreground"
            title={facets.qualifiedName}
          >
            {facets.qualifiedName}
          </code>
          <Button
            size="icon-xs"
            variant="ghost"
            aria-label="Copy qualified name"
            onClick={() => onCopyName(facets.qualifiedName)}
          >
            <HugeiconsIcon icon={Copy01Icon} strokeWidth={2} />
          </Button>
        </div>
      </div>

      {detail === null ? (
        loading ? (
          <div
            role="status"
            aria-label="Loading properties"
            className="flex flex-col gap-2"
          >
            <Skeleton className="h-4 w-2/3" />
            <Skeleton className="h-4 w-1/2" />
            <Skeleton className="h-4 w-3/4" />
          </div>
        ) : (
          <p className="text-xs text-muted-foreground">
            The description of this object has not been read yet. Open its
            Structure to load it.
          </p>
        )
      ) : (
        <dl className="grid grid-cols-[max-content_1fr] gap-x-3 gap-y-1.5 text-xs">
          <dt className="text-muted-foreground">Rows</dt>
          <dd>
            {detail.estimatedRows !== null
              ? `~${detail.estimatedRows.toLocaleString("en-US")} (estimate)`
              : "Not reported"}
          </dd>
          <dt className="text-muted-foreground">Size</dt>
          <dd>
            {detail.sizeBytes !== null
              ? sizeLabel(detail.sizeBytes)
              : "Not reported"}
          </dd>
          <dt className="text-muted-foreground">Columns</dt>
          <dd>{detail.fields.length.toLocaleString("en-US")}</dd>
          <dt className="text-muted-foreground">Primary key</dt>
          <dd dir="auto" className="font-mono break-words">
            {primaryKey.length > 0
              ? primaryKey.map((field) => field.name).join(", ")
              : "None declared"}
          </dd>
          {facets.indexes !== null ? (
            <>
              <dt className="text-muted-foreground">Indexes</dt>
              <dd>{facets.indexes.length.toLocaleString("en-US")}</dd>
            </>
          ) : null}
          {facets.foreignKeys !== null ? (
            <>
              <dt className="text-muted-foreground">Foreign keys</dt>
              <dd>{facets.foreignKeys.length.toLocaleString("en-US")}</dd>
            </>
          ) : null}
          <dt className="text-muted-foreground">Comment</dt>
          <dd data-selectable className="break-words">
            {detail.comment ?? (
              <span className="text-muted-foreground">None</span>
            )}
          </dd>
        </dl>
      )}
    </div>
  )
}
