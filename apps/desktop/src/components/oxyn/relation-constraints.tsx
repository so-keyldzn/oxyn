import { HugeiconsIcon } from "@hugeicons/react"
import { Copy01Icon } from "@hugeicons/core-free-icons"

import { Badge } from "@/components/ui/badge"
import { Button } from "@/components/ui/button"
import { Separator } from "@/components/ui/separator"
import {
  Table,
  TableBody,
  TableCell,
  TableHead,
  TableHeader,
  TableRow,
} from "@/components/ui/table"
import type { ConstraintRow, IndexRow } from "@/lib/ipc/metadata"
import type { RelationDetail } from "@/lib/ipc/types"

/** `primary_key` → `PRIMARY KEY`, as the constraint list writes it. */
export function constraintKind(kind: string) {
  return kind.replaceAll("_", " ").toUpperCase()
}

export function validationLabel(validated: boolean | null) {
  if (validated === null) return "Not reported"
  return validated ? "Validated" : "Not validated"
}

/**
 * The constraints of a relation, and what the relation read already says
 * about nullability and unique indexes.
 *
 * A definition is the engine's rendering: shown, copied, never executed. The
 * two summaries are omitted when the relation itself was not read, rather
 * than claiming « none ».
 */
export function RelationConstraints({
  constraints,
  detail,
  indexes,
  onCopyDefinition,
}: {
  constraints: Array<ConstraintRow>
  detail: RelationDetail | null
  indexes: Array<IndexRow> | null
  onCopyDefinition: (text: string) => void
}) {
  const notNull = detail?.fields.filter((field) => !field.nullable) ?? []
  const unique = indexes?.filter((index) => index.unique) ?? []

  return (
    <div className="flex flex-col">
      <Table>
        <TableHeader className="sticky top-0 bg-card">
          <TableRow>
            <TableHead>Constraint</TableHead>
            <TableHead>Type</TableHead>
            <TableHead>Columns</TableHead>
            <TableHead>Status</TableHead>
            <TableHead>Definition</TableHead>
          </TableRow>
        </TableHeader>
        <TableBody>
          {constraints.map((constraint, position) => (
            <TableRow key={`${position}:${constraint.name}`}>
              {/* Names come from the database: they carry their own
                  direction and are never markup. */}
              <TableCell dir="auto" className="font-medium">
                {constraint.name || (
                  <span className="text-muted-foreground">Unnamed</span>
                )}
              </TableCell>
              <TableCell>
                <Badge variant="outline">
                  {constraintKind(constraint.kind)}
                </Badge>
              </TableCell>
              <TableCell dir="auto" className="font-mono text-xs">
                {constraint.fields.length > 0
                  ? constraint.fields.join(", ")
                  : "Not reported"}
              </TableCell>
              <TableCell className="text-muted-foreground">
                {validationLabel(constraint.validated)}
              </TableCell>
              <TableCell className="max-w-96">
                {constraint.expression ? (
                  <span className="flex min-w-0 items-center gap-1">
                    <code
                      data-selectable
                      className="truncate font-mono text-xs"
                      title={constraint.expression}
                    >
                      {constraint.expression}
                    </code>
                    <Button
                      size="icon-xs"
                      variant="ghost"
                      aria-label={`Copy the definition of ${constraint.name || "this constraint"}`}
                      onClick={() =>
                        onCopyDefinition(constraint.expression ?? "")
                      }
                    >
                      <HugeiconsIcon icon={Copy01Icon} strokeWidth={2} />
                    </Button>
                  </span>
                ) : null}
              </TableCell>
            </TableRow>
          ))}
        </TableBody>
      </Table>

      {detail ? (
        <>
          <Separator />
          <dl className="grid gap-x-4 gap-y-1 px-4 py-3 text-xs sm:grid-cols-[max-content_1fr]">
            <dt className="font-medium">NOT NULL columns</dt>
            <dd className="text-muted-foreground">
              {notNull.length > 0
                ? notNull.map((field) => field.name).join(" · ")
                : "None: every column accepts NULL."}
            </dd>
            {indexes ? (
              <>
                <dt className="font-medium">Unique indexes</dt>
                <dd className="text-muted-foreground">
                  {unique.length > 0
                    ? `${unique.map((index) => index.name || "Unnamed").join(" · ")} · listed under Indexes.`
                    : "None. Unique indexes, when there are any, are listed under Indexes."}
                </dd>
              </>
            ) : null}
          </dl>
        </>
      ) : null}
    </div>
  )
}
