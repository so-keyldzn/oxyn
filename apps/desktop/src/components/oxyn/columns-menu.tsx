import { HugeiconsIcon } from "@hugeicons/react"
import { LayoutThreeColumnIcon, ViewIcon } from "@hugeicons/core-free-icons"

import { Button } from "@/components/ui/button"
import {
  DropdownMenu,
  DropdownMenuCheckboxItem,
  DropdownMenuContent,
  DropdownMenuGroup,
  DropdownMenuItem,
  DropdownMenuLabel,
  DropdownMenuSeparator,
  DropdownMenuSub,
  DropdownMenuSubContent,
  DropdownMenuSubTrigger,
  DropdownMenuTrigger,
} from "@/components/ui/dropdown-menu"
import type { ResultColumn } from "@/lib/ipc/types"

/** « 5 of 7 columns shown ». Pure, so it is tested. */
export function columnsSummary(total: number, hidden: number) {
  const shown = total - hidden
  return hidden === 0
    ? `All ${total.toLocaleString("en-US")} ${total === 1 ? "column" : "columns"} shown`
    : `${shown.toLocaleString("en-US")} of ${total.toLocaleString("en-US")} columns shown`
}

/**
 * Which columns of a result the grid draws (docs/UX-SPEC.md, « Colonnes et
 * inspection des valeurs »).
 *
 * The choice is local to the view: it keeps every Arrow index, every row
 * received and the SQL as they are, and the export still writes every column —
 * which the menu says, so that nobody hides `email` expecting it to stay out of
 * the file. The last shown column cannot be hidden: a grid with no column is
 * a result that looks empty.
 */
export function ColumnsMenu(props: ColumnsMenuProps) {
  return (
    <DropdownMenu>
      {/* `xs`, as Cancel beside it: a taller button grows the footer and
          takes a row from the grid over the console's splitter floor. */}
      <DropdownMenuTrigger render={<Button variant="outline" size="xs" />}>
        <ColumnsLabel {...props} />
      </DropdownMenuTrigger>
      <DropdownMenuContent align="end" className="max-h-96 w-72">
        <ColumnsItems {...props} />
      </DropdownMenuContent>
    </DropdownMenu>
  )
}

/**
 * The same choice as a submenu of `Actions`, below 1200 px, where the footer
 * has no room for its own button (docs/UX-SPEC.md, « Largeur réduite »).
 */
export function ColumnsSubmenu(props: ColumnsMenuProps) {
  return (
    <DropdownMenuSub>
      <DropdownMenuSubTrigger>
        <ColumnsLabel {...props} />
      </DropdownMenuSubTrigger>
      <DropdownMenuSubContent className="max-h-96 w-72">
        <ColumnsItems {...props} />
      </DropdownMenuSubContent>
    </DropdownMenuSub>
  )
}

interface ColumnsMenuProps {
  columns: ReadonlyArray<ResultColumn>
  /** Arrow indexes of the hidden columns. */
  hidden: ReadonlySet<number>
  onHiddenChange: (hidden: ReadonlySet<number>) => void
}

function countHidden({ columns, hidden }: ColumnsMenuProps) {
  return columns.reduce(
    (count, _, index) => count + (hidden.has(index) ? 1 : 0),
    0
  )
}

function ColumnsLabel(props: ColumnsMenuProps) {
  const hiddenCount = countHidden(props)
  const total = props.columns.length
  return (
    <>
      <HugeiconsIcon
        icon={LayoutThreeColumnIcon}
        strokeWidth={2}
        data-icon="inline-start"
      />
      Columns
      {hiddenCount > 0 ? (
        <span className="text-muted-foreground tabular-nums">
          {total - hiddenCount}/{total}
        </span>
      ) : null}
    </>
  )
}

function ColumnsItems(props: ColumnsMenuProps) {
  const { columns, hidden, onHiddenChange } = props
  const hiddenCount = countHidden(props)
  const lastShown = columns.length - hiddenCount === 1

  const toggle = (index: number, shown: boolean) => {
    const next = new Set(hidden)
    if (shown) next.delete(index)
    else next.add(index)
    onHiddenChange(next)
  }

  return (
    <>
      <DropdownMenuGroup>
        <DropdownMenuLabel className="tabular-nums">
          {columnsSummary(columns.length, hiddenCount)}
        </DropdownMenuLabel>
        {columns.map((column, index) => {
          const shown = !hidden.has(index)
          return (
            <DropdownMenuCheckboxItem
              key={index}
              checked={shown}
              disabled={shown && lastShown}
              onCheckedChange={(checked) => toggle(index, checked)}
            >
              <span dir="auto" className="min-w-0 flex-1 truncate">
                {column.name}
              </span>
              <span className="max-w-24 shrink-0 truncate font-mono text-xs text-muted-foreground">
                {column.dataType}
              </span>
            </DropdownMenuCheckboxItem>
          )
        })}
      </DropdownMenuGroup>
      <DropdownMenuSeparator />
      <DropdownMenuGroup>
        <DropdownMenuItem
          disabled={hiddenCount === 0}
          closeOnClick={false}
          onClick={() => onHiddenChange(new Set())}
        >
          <HugeiconsIcon icon={ViewIcon} strokeWidth={2} />
          Show all
        </DropdownMenuItem>
        <DropdownMenuLabel className="font-normal text-muted-foreground">
          Hiding changes this view only. An export keeps every column.
        </DropdownMenuLabel>
      </DropdownMenuGroup>
    </>
  )
}
