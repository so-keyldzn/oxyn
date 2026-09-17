import { HugeiconsIcon } from "@hugeicons/react"
import { LinkSquare02Icon, SourceCodeIcon } from "@hugeicons/core-free-icons"

import { addressLabel } from "@/components/oxyn/catalog-tree"
import { Badge } from "@/components/ui/badge"
import { Button } from "@/components/ui/button"
import {
  Table,
  TableBody,
  TableCell,
  TableHead,
  TableHeader,
  TableRow,
} from "@/components/ui/table"
import type { ForeignKeyRow, IncomingKeyRow } from "@/lib/ipc/metadata"
import type { CatalogAddress } from "@/lib/ipc/types"

export function cardinalityLabel(sourceUnique: boolean | null) {
  if (sourceUnique === null) return "Not reported"
  return sourceUnique ? "One to one" : "Many to one"
}

function RowActions({
  openLabel,
  target,
  onOpen,
  onReview,
}: {
  openLabel: string
  target: CatalogAddress
  onOpen?: (address: CatalogAddress) => void
  onReview?: () => void
}) {
  return (
    <span className="flex items-center justify-end gap-1">
      {onOpen ? (
        <Button
          size="xs"
          variant="ghost"
          onClick={() => onOpen(target)}
          // One row per key: « Open referenced table » ten times over tells a
          // screen reader nothing about which one it is opening.
          aria-label={`${openLabel} ${addressLabel(target)}`}
        >
          <HugeiconsIcon
            icon={LinkSquare02Icon}
            strokeWidth={2}
            data-icon="inline-start"
          />
          {openLabel}
        </Button>
      ) : null}
      {onReview ? (
        <Button size="xs" variant="ghost" onClick={onReview}>
          <HugeiconsIcon
            icon={SourceCodeIcon}
            strokeWidth={2}
            data-icon="inline-start"
          />
          Review related-row query
        </Button>
      ) : null}
    </span>
  )
}

/**
 * Foreign keys this relation declares towards others.
 *
 * Opening a referenced table selects it like a click in the explorer; the
 * related-row query opens in a new console with blank conditions and is never
 * run by opening it.
 */
export function OutgoingKeys({
  keys,
  onOpen,
  onReviewRelatedRows,
}: {
  keys: Array<ForeignKeyRow>
  onOpen?: (address: CatalogAddress) => void
  onReviewRelatedRows?: (index: number) => void
}) {
  return (
    <Table>
      <TableHeader className="sticky top-0 bg-card">
        <TableRow>
          <TableHead>Foreign key</TableHead>
          <TableHead>From columns</TableHead>
          <TableHead>Referenced object / columns</TableHead>
          <TableHead>On delete</TableHead>
          <TableHead>
            <span className="sr-only">Actions</span>
          </TableHead>
        </TableRow>
      </TableHeader>
      <TableBody>
        {keys.map((key, index) => (
          <TableRow key={`${index}:${key.name}`}>
            {/* Names come from the database: their own direction, as text. */}
            <TableCell dir="auto" className="font-medium">
              {key.name || (
                <span className="text-muted-foreground">Unnamed</span>
              )}
            </TableCell>
            <TableCell dir="auto" className="font-mono text-xs">
              {key.fields.join(", ")}
            </TableCell>
            <TableCell dir="auto" className="font-mono text-xs">
              {addressLabel(key.references)} ({key.referencedFields.join(", ")})
            </TableCell>
            <TableCell>
              {key.cascades ? (
                <Badge variant="outline">{key.onDelete}</Badge>
              ) : (
                <span className="text-muted-foreground">{key.onDelete}</span>
              )}
            </TableCell>
            <TableCell>
              <RowActions
                openLabel="Open referenced table"
                target={key.references}
                onOpen={onOpen}
                onReview={
                  onReviewRelatedRows
                    ? () => onReviewRelatedRows(index)
                    : undefined
                }
              />
            </TableCell>
          </TableRow>
        ))}
      </TableBody>
    </Table>
  )
}

/**
 * Foreign keys other relations declare towards this one.
 *
 * Cardinality comes from unique keys on the source; it is not a count and not
 * an integrity check.
 */
export function IncomingKeys({
  keys,
  onOpen,
  onReviewRelatedRows,
}: {
  keys: Array<IncomingKeyRow>
  onOpen?: (address: CatalogAddress) => void
  onReviewRelatedRows?: (index: number) => void
}) {
  return (
    <Table>
      <TableHeader className="sticky top-0 bg-card">
        <TableRow>
          <TableHead>From</TableHead>
          <TableHead>To</TableHead>
          <TableHead>Cardinality</TableHead>
          <TableHead>
            <span className="sr-only">Actions</span>
          </TableHead>
        </TableRow>
      </TableHeader>
      <TableBody>
        {keys.map((incoming, index) => (
          <TableRow key={`${index}:${incoming.key.name}`}>
            <TableCell dir="auto" className="font-mono text-xs">
              {addressLabel(incoming.source)} ({incoming.key.fields.join(", ")})
            </TableCell>
            <TableCell dir="auto" className="font-mono text-xs">
              ({incoming.key.referencedFields.join(", ")})
            </TableCell>
            <TableCell className="text-muted-foreground">
              {cardinalityLabel(incoming.sourceUnique)}
            </TableCell>
            <TableCell>
              <RowActions
                openLabel="Open source table"
                target={incoming.source}
                onOpen={onOpen}
                onReview={
                  onReviewRelatedRows
                    ? () => onReviewRelatedRows(index)
                    : undefined
                }
              />
            </TableCell>
          </TableRow>
        ))}
      </TableBody>
    </Table>
  )
}
