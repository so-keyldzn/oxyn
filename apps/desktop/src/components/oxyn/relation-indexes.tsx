import { Badge } from "@/components/ui/badge"
import {
  Table,
  TableBody,
  TableCell,
  TableHead,
  TableHeader,
  TableRow,
} from "@/components/ui/table"
import type { IndexRow } from "@/lib/ipc/metadata"

/**
 * The indexes of a relation, as its description reported them.
 *
 * Read with the relation itself (`CatalogRefreshScope::Relation`): the frame
 * around it is the Structure facet's. An index predicate is shown as text and
 * never run.
 */
export function RelationIndexes({ indexes }: { indexes: Array<IndexRow> }) {
  return (
    <Table>
      <TableHeader className="sticky top-0 bg-card">
        <TableRow>
          <TableHead>Index</TableHead>
          <TableHead>Columns</TableHead>
          <TableHead>Uniqueness</TableHead>
          <TableHead>Method</TableHead>
          <TableHead>Partial on</TableHead>
        </TableRow>
      </TableHeader>
      <TableBody>
        {indexes.map((index, position) => (
          <TableRow key={`${position}:${index.name}`}>
            {/* Names come from the database: they carry their own direction
                and are never markup. */}
            <TableCell dir="auto" className="font-medium">
              {index.name || (
                <span className="text-muted-foreground">Unnamed</span>
              )}
            </TableCell>
            <TableCell dir="auto" className="font-mono text-xs">
              {index.fields.length > 0 ? index.fields.join(", ") : "—"}
            </TableCell>
            <TableCell>
              {index.unique ? (
                <Badge variant="outline">Unique</Badge>
              ) : (
                <span className="text-muted-foreground">Non-unique</span>
              )}
            </TableCell>
            <TableCell className="text-muted-foreground">
              {index.method ?? "Not reported"}
            </TableCell>
            <TableCell className="font-mono text-xs text-muted-foreground">
              {index.predicate ?? ""}
            </TableCell>
          </TableRow>
        ))}
      </TableBody>
    </Table>
  )
}
