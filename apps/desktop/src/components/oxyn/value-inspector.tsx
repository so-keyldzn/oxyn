import * as React from "react"
import { HugeiconsIcon } from "@hugeicons/react"
import { ViewIcon } from "@hugeicons/core-free-icons"

import { CellValue } from "@/components/oxyn/cell-value"
import { Button } from "@/components/ui/button"
import {
  Empty,
  EmptyDescription,
  EmptyHeader,
  EmptyMedia,
  EmptyTitle,
} from "@/components/ui/empty"
import { Skeleton } from "@/components/ui/skeleton"
import type { Cell, ResultColumn } from "@/lib/ipc/types"
import { cn } from "@/lib/utils"

/** The row the inspector follows. `cells` is null while its page loads. */
export interface InspectedRow {
  row: number
  columns: Array<ResultColumn>
  cells: Array<Cell> | null
}

/**
 * The record inspector: the fields of the selected row, read-only
 * (docs/UX-SPEC.md, « Colonnes et inspection des valeurs »).
 *
 * It follows the grid's selection and reads nothing itself: a missing page is
 * loaded from the existing result, never by a new query. Arrows move between
 * fields; Enter opens the full value.
 */
export function ValueInspector({
  target,
  column,
  onSelectColumn,
  onInspect,
}: {
  target: InspectedRow | null
  column: number | null
  onSelectColumn: (column: number) => void
  onInspect: (column: number) => void
}) {
  const listRef = React.useRef<HTMLDivElement>(null)
  const baseId = React.useId()
  const fieldId = (index: number) => `${baseId}-f${index}`

  React.useEffect(() => {
    if (column === null) return
    listRef.current
      ?.querySelector<HTMLElement>(`[data-column="${column}"]`)
      ?.scrollIntoView({ block: "nearest" })
  }, [column])

  if (!target) {
    return (
      <Empty className="h-full border-0">
        <EmptyHeader>
          <EmptyMedia variant="icon">
            <HugeiconsIcon icon={ViewIcon} strokeWidth={2} />
          </EmptyMedia>
          <EmptyTitle>Record inspector</EmptyTitle>
          <EmptyDescription>
            Select a row in the result grid to read its fields here.
          </EmptyDescription>
        </EmptyHeader>
      </Empty>
    )
  }

  const count = target.columns.length
  const onKeyDown = (event: React.KeyboardEvent) => {
    if (count === 0) return
    const current = column ?? 0
    switch (event.key) {
      case "ArrowDown":
        onSelectColumn(Math.min(count - 1, current + 1))
        break
      case "ArrowUp":
        onSelectColumn(Math.max(0, current - 1))
        break
      case "Home":
        onSelectColumn(0)
        break
      case "End":
        onSelectColumn(count - 1)
        break
      case "Enter":
        onInspect(current)
        break
      default:
        return
    }
    event.preventDefault()
  }

  return (
    <div className="flex h-full min-h-0 flex-col">
      <div className="flex h-10 shrink-0 items-center gap-2 border-b px-3">
        <span className="truncate text-sm font-medium">
          Record · row {(target.row + 1).toLocaleString("en-US")}
        </span>
        <span className="text-xs text-muted-foreground">Read only</span>
        <Button
          size="xs"
          variant="outline"
          className="ml-auto"
          disabled={column === null}
          onClick={() => column !== null && onInspect(column)}
        >
          Inspect full value
        </Button>
      </div>
      <div
        ref={listRef}
        role="listbox"
        aria-label={`Fields of row ${target.row + 1}`}
        // The box keeps the focus while the arrows move the selection: without
        // this, a screen reader is never told which field is selected.
        aria-activedescendant={column === null ? undefined : fieldId(column)}
        tabIndex={0}
        onKeyDown={onKeyDown}
        className="min-h-0 flex-1 overflow-auto outline-none focus-visible:ring-2 focus-visible:ring-ring focus-visible:ring-inset"
      >
        {target.columns.map((field, index) => (
          <div
            key={`${index}:${field.name}`}
            id={fieldId(index)}
            role="option"
            aria-selected={column === index}
            data-column={index}
            onClick={() => onSelectColumn(index)}
            onDoubleClick={() => onInspect(index)}
            className={cn(
              "flex flex-col gap-0.5 border-b px-3 py-2",
              column === index && "bg-accent"
            )}
          >
            <span className="truncate text-xs text-muted-foreground">
              <span dir="auto" className="font-medium text-foreground">
                {field.name}
              </span>{" "}
              · <span className="font-mono">{field.dataType}</span>
            </span>
            <span className="min-w-0 font-mono text-xs">
              {target.cells ? (
                <CellValue cell={target.cells[index] ?? null} />
              ) : (
                <Skeleton className="h-3 w-2/3" />
              )}
            </span>
          </div>
        ))}
      </div>
    </div>
  )
}
