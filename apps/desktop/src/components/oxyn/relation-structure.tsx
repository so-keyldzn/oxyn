import {
  flexRender,
  getCoreRowModel,
  getSortedRowModel,
  useReactTable,
} from "@tanstack/react-table"
import type { ColumnDef, SortingState } from "@tanstack/react-table"
import { useVirtualizer } from "@tanstack/react-virtual"
import * as React from "react"
import { HugeiconsIcon } from "@hugeicons/react"
import {
  Alert02Icon,
  ArrowReloadHorizontalIcon,
  Key01Icon,
  LockIcon,
  TableIcon,
} from "@hugeicons/core-free-icons"

import { Alert, AlertDescription, AlertTitle } from "@/components/ui/alert"
import { Button } from "@/components/ui/button"
import {
  Empty,
  EmptyContent,
  EmptyDescription,
  EmptyHeader,
  EmptyMedia,
  EmptyTitle,
} from "@/components/ui/empty"
import { Skeleton } from "@/components/ui/skeleton"
import { Spinner } from "@/components/ui/spinner"
import {
  TableBody,
  TableCell,
  TableHead,
  TableHeader,
  TableRow,
} from "@/components/ui/table"
import type { RelationDetail, RelationField } from "@/lib/ipc/types"

/** A structure row: one line of text, so the virtual list can size it. */
const ROW_HEIGHT = 32

const columns: Array<ColumnDef<RelationField>> = [
  {
    accessorKey: "position",
    header: "#",
    cell: ({ getValue }) => (
      <span className="text-muted-foreground tabular-nums">
        {getValue<number>()}
      </span>
    ),
  },
  {
    accessorKey: "name",
    header: "Column",
    cell: ({ row }) => (
      <span className="flex max-w-80 min-w-0 items-center gap-1.5 font-medium">
        {row.original.primaryKey ? (
          <HugeiconsIcon
            icon={Key01Icon}
            strokeWidth={2}
            className="size-3.5 shrink-0 text-warning"
            aria-label="Primary key"
          />
        ) : null}
        <span dir="auto" className="truncate" title={row.original.name}>
          {row.original.name}
        </span>
      </span>
    ),
  },
  {
    accessorKey: "rawType",
    header: "Type",
    cell: ({ row }) => (
      <span
        className="block max-w-64 truncate font-mono text-xs"
        title={`${row.original.rawType} · ${row.original.logicalType}`}
      >
        {row.original.rawType}
      </span>
    ),
  },
  {
    accessorKey: "nullable",
    header: "Null",
    // The same typographic weight both ways: a constraint is read, not
    // spotted by a badge next to plain text.
    cell: ({ getValue }) => (
      <span
        className={
          getValue<boolean>()
            ? "font-mono text-xs text-muted-foreground"
            : "font-mono text-xs text-foreground"
        }
      >
        {getValue<boolean>() ? "NULL" : "NOT NULL"}
      </span>
    ),
  },
  {
    accessorKey: "default",
    header: "Default",
    cell: ({ getValue }) => (
      <span
        className="block max-w-64 truncate font-mono text-xs text-muted-foreground"
        title={getValue<string | null>() ?? undefined}
      >
        {getValue<string | null>() ?? ""}
      </span>
    ),
  },
  {
    accessorKey: "comment",
    header: "Comment",
    cell: ({ getValue }) => (
      <span
        dir="auto"
        className="block max-w-96 truncate text-muted-foreground"
        title={getValue<string | null>() ?? undefined}
      >
        {getValue<string | null>() ?? ""}
      </span>
    ),
  },
]

/**
 * A relation's columns, as the catalog loaded them.
 *
 * A failed read and a refused one are said as such, with the server's words
 * and whether trying again can help; neither is « not loaded »
 * (docs/UX-SPEC.md, « États d'une vue »). A structure already read stays on
 * screen under a failed refresh, marked as possibly outdated. The list is
 * virtualized: a relation of a thousand columns renders the visible ones.
 */
export function RelationStructure({
  detail,
  error = null,
  denied = null,
  refreshing = false,
  onRefresh,
}: {
  /** `undefined` while the first read runs, `null` when never read. */
  detail: RelationDetail | null | undefined
  /** The last read failed; `retryable` comes from the backend. */
  error?: { message: string; retryable: boolean } | null
  /** The connection policy refused the read: trying again changes nothing. */
  denied?: string | null
  refreshing?: boolean
  onRefresh?: () => void
}) {
  const [sorting, setSorting] = React.useState<SortingState>([])
  const scrollRef = React.useRef<HTMLDivElement>(null)
  const table = useReactTable({
    data: detail?.fields ?? EMPTY,
    columns,
    state: { sorting },
    onSortingChange: setSorting,
    getCoreRowModel: getCoreRowModel(),
    getSortedRowModel: getSortedRowModel(),
  })
  const rows = table.getRowModel().rows
  const virtualizer = useVirtualizer({
    count: rows.length,
    getScrollElement: () => scrollRef.current,
    estimateSize: () => ROW_HEIGHT,
    overscan: 16,
  })

  const refreshButton = onRefresh ? (
    <Button
      size="sm"
      variant="outline"
      onClick={onRefresh}
      disabled={refreshing}
    >
      {refreshing ? (
        <Spinner data-icon="inline-start" />
      ) : (
        <HugeiconsIcon
          icon={ArrowReloadHorizontalIcon}
          strokeWidth={2}
          data-icon="inline-start"
        />
      )}
      Refresh structure
    </Button>
  ) : null

  const problem = denied ? (
    <Alert>
      <HugeiconsIcon icon={LockIcon} strokeWidth={2} />
      <AlertTitle>Reading the structure was refused</AlertTitle>
      <AlertDescription>
        <p data-selectable dir="auto">
          {denied}
        </p>
        <p className="text-xs">
          The connection policy decides this read; trying again as is gives the
          same answer.
        </p>
      </AlertDescription>
    </Alert>
  ) : error ? (
    <Alert variant="destructive">
      <HugeiconsIcon icon={Alert02Icon} strokeWidth={2} />
      <AlertTitle>The structure could not be read</AlertTitle>
      <AlertDescription className="flex flex-col gap-2">
        {/* The server's words, code included — never a paraphrase. */}
        <pre
          data-selectable
          // A long identifier must not widen the Alert's grid column.
          className="font-mono text-xs wrap-anywhere whitespace-pre-wrap text-foreground"
        >
          {error.message}
        </pre>
        <p className="text-xs">
          {error.retryable
            ? "This error is transient: refreshing may succeed."
            : "Refreshing as is will fail the same way."}
          {detail ? " The columns below come from an earlier read." : ""}
        </p>
        {refreshButton ? <div>{refreshButton}</div> : null}
      </AlertDescription>
    </Alert>
  ) : null

  if (!detail) {
    if (problem) return <div className="p-4">{problem}</div>
    if (detail === undefined || refreshing) {
      return (
        <div
          role="status"
          aria-label="Loading structure"
          className="flex flex-col gap-2 p-4"
        >
          {Array.from({ length: 6 }, (_, index) => (
            <Skeleton key={index} className="h-6 w-full" />
          ))}
        </div>
      )
    }
    return (
      <Empty className="h-full border-0">
        <EmptyHeader>
          <EmptyMedia variant="icon">
            <HugeiconsIcon icon={TableIcon} strokeWidth={2} />
          </EmptyMedia>
          <EmptyTitle>Structure not read yet</EmptyTitle>
          <EmptyDescription>
            Oxyn reads a relation&apos;s columns when asked, from the catalog of
            this connection.
          </EmptyDescription>
        </EmptyHeader>
        {refreshButton ? <EmptyContent>{refreshButton}</EmptyContent> : null}
      </Empty>
    )
  }

  const items = virtualizer.getVirtualItems()
  const before = items[0]?.start ?? 0
  const after = virtualizer.getTotalSize() - (items.at(-1)?.end ?? 0)

  return (
    <div className="flex h-full min-h-0 flex-col">
      <div className="flex flex-wrap items-center gap-x-3 gap-y-1 border-b px-4 py-2 text-xs text-muted-foreground">
        <span className="tabular-nums">
          {detail.fields.length.toLocaleString("en-US")}{" "}
          {detail.fields.length === 1 ? "column" : "columns"}
        </span>
        {detail.estimatedRows != null ? (
          <span className="tabular-nums">
            ~{detail.estimatedRows.toLocaleString("en-US")} rows (estimate)
          </span>
        ) : null}
        {detail.comment ? (
          <span data-selectable dir="auto" className="min-w-0 truncate">
            {detail.comment}
          </span>
        ) : null}
        {refreshing ? (
          <span role="status" className="ms-auto flex items-center gap-1.5">
            <Spinner aria-hidden className="size-3" /> Refreshing…
          </span>
        ) : onRefresh && !problem ? (
          <Button
            size="xs"
            variant="ghost"
            className="ms-auto"
            onClick={onRefresh}
          >
            <HugeiconsIcon
              icon={ArrowReloadHorizontalIcon}
              strokeWidth={2}
              data-icon="inline-start"
            />
            Refresh structure
          </Button>
        ) : null}
      </div>
      {problem ? <div className="border-b p-3">{problem}</div> : null}
      <div ref={scrollRef} className="min-h-0 flex-1 overflow-auto">
        {/* A bare table: the shadcn container scrolls on its own, and a sticky
            header must stick to this scroller. */}
        <table
          data-slot="table"
          aria-rowcount={rows.length + 1}
          className="w-full caption-bottom text-sm"
        >
          <TableHeader className="sticky top-0 z-10 bg-card">
            {table.getHeaderGroups().map((group) => (
              <TableRow key={group.id} aria-rowindex={1}>
                {group.headers.map((header) => (
                  <TableHead
                    key={header.id}
                    aria-sort={
                      header.column.getIsSorted() === "asc"
                        ? "ascending"
                        : header.column.getIsSorted() === "desc"
                          ? "descending"
                          : "none"
                    }
                  >
                    <button
                      type="button"
                      className="rounded-sm outline-none hover:text-foreground focus-visible:ring-2 focus-visible:ring-ring"
                      onClick={header.column.getToggleSortingHandler()}
                    >
                      {flexRender(
                        header.column.columnDef.header,
                        header.getContext()
                      )}
                    </button>
                  </TableHead>
                ))}
              </TableRow>
            ))}
          </TableHeader>
          <TableBody>
            {before > 0 ? (
              <tr aria-hidden style={{ height: before }}>
                <td colSpan={columns.length} />
              </tr>
            ) : null}
            {items.map((item) => {
              const row = rows[item.index]
              if (!row) return null
              return (
                <TableRow
                  key={row.id}
                  aria-rowindex={item.index + 2}
                  style={{ height: ROW_HEIGHT }}
                >
                  {row.getVisibleCells().map((cell) => (
                    <TableCell key={cell.id} className="py-0">
                      {flexRender(
                        cell.column.columnDef.cell,
                        cell.getContext()
                      )}
                    </TableCell>
                  ))}
                </TableRow>
              )
            })}
            {after > 0 ? (
              <tr aria-hidden style={{ height: after }}>
                <td colSpan={columns.length} />
              </tr>
            ) : null}
          </TableBody>
        </table>
      </div>
    </div>
  )
}

const EMPTY: Array<RelationField> = []
