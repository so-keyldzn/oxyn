import * as React from "react"
import { HugeiconsIcon } from "@hugeicons/react"
import {
  ArrowDown01Icon,
  ArrowLeft01Icon,
  ArrowRight01Icon,
  ArrowUp01Icon,
  Delete02Icon,
  Sorting05Icon,
} from "@hugeicons/core-free-icons"
import { cn } from "cn"

import { Badge } from "@/components/ui/badge"
import { Button } from "@/components/ui/button"
import {
  InputGroup,
  InputGroupAddon,
  InputGroupButton,
  InputGroupInput,
  InputGroupText,
} from "@/components/ui/input-group"
import { NativeSelect, NativeSelectOption } from "@/components/ui/native-select"
import {
  Popover,
  PopoverContent,
  PopoverDescription,
  PopoverHeader,
  PopoverTitle,
  PopoverTrigger,
} from "@/components/ui/popover"
import { Spinner } from "@/components/ui/spinner"
import { MAX_SORT_KEYS, PREVIEW_ROWS } from "@/lib/ipc/metadata"
import type {
  Pagination,
  PreviewShape,
  PreviewSortKey,
} from "@/lib/ipc/metadata"

/** The state of the preview area, as the notice needs it. */
export type PreviewStatus =
  "initial" | "loading" | "loaded" | "empty" | "failed"

/** Whether the field holds something else than what produced the rows. */
export function draftDiffers(draft: string, applied: string | null) {
  return draft.trim() !== (applied ?? "").trim()
}

/**
 * The sentence under the bar: what it cannot say in its own width. Pure, so it
 * is tested without a DOM. The wording is the GPUI workspace's.
 */
export function previewNotice({
  status,
  pending,
  filtered,
  pagination,
  applied,
}: {
  status: PreviewStatus
  pending: boolean
  filtered: boolean
  pagination: Pagination | null
  applied: PreviewShape
}): { text: string; warning: boolean } | null {
  switch (status) {
    case "failed":
      return {
        text: "This read was refused. The rows of the previous shape were not kept under the new one, and the text stays where it can be fixed.",
        warning: true,
      }
    case "empty":
      return filtered
        ? {
            text: "No row matches this filter. The read succeeded; the table may still hold rows.",
            warning: false,
          }
        : { text: "This table holds no rows.", warning: false }
    case "loading":
      return pending
        ? {
            text: "Reading… This filter is not in force until the server answers.",
            warning: false,
          }
        : null
    default:
      break
  }
  if (pending) {
    return {
      text: "This filter is not applied yet. Apply runs a new read against the server; the rows below still come from the shape in force.",
      warning: false,
    }
  }
  if (status !== "loaded" || !pagination) return null
  switch (pagination.type) {
    case "needsOrder":
      return {
        text: "Rows come in no guaranteed order. Sort the preview to browse it page by page: an OFFSET over an uncertain order shows a row twice and hides another, with nothing to say so.",
        warning: false,
      }
    case "noUniqueKey":
      return {
        text: "This preview stays on its first page: no unique key is known for this relation, so no order can be made total.",
        warning: false,
      }
    case "ready":
      return pagination.previous
        ? {
            text: `Rows ${pagination.firstRow.toLocaleString("en-US")}–${(applied.offset + PREVIEW_ROWS).toLocaleString("en-US")} of this order. Each page is a new read: the server may have changed between two of them.`,
            warning: false,
          }
        : null
  }
}

function SortComposer({
  columns,
  applied,
  loadingColumns,
  onLoadColumns,
  onApply,
}: {
  columns: Array<string> | null
  applied: Array<PreviewSortKey>
  loadingColumns: boolean
  onLoadColumns: () => void
  onApply: (sort: Array<PreviewSortKey>) => void
}) {
  const [open, setOpen] = React.useState(false)
  const [draft, setDraft] = React.useState<Array<PreviewSortKey>>(applied)
  const keyList = React.useRef<HTMLOListElement>(null)
  // Moving a key to an end disables the button that was just pressed, and the
  // focus would fall back to the body. It follows the row instead.
  const follow = React.useRef<{ index: number; way: "up" | "down" } | null>(
    null
  )
  React.useEffect(() => {
    const target = follow.current
    follow.current = null
    if (!target) return
    const row = `[data-index="${target.index}"]:not(:disabled)`
    const moved =
      keyList.current?.querySelector<HTMLButtonElement>(
        `[data-move="${target.way}"]${row}`
      ) ??
      // At an end that button is now disabled: the other one keeps the focus
      // on the row that just moved, rather than losing it to the body.
      keyList.current?.querySelector<HTMLButtonElement>(`[data-move]${row}`)
    moved?.focus()
  }, [draft])

  React.useEffect(() => {
    // The menu shows what produced the rows each time it opens: an order the
    // server never answered is not kept as if it were in force.
    if (open) setDraft(applied)
  }, [open, applied])

  const unused = (columns ?? []).filter(
    (column) => !draft.some((key) => key.column === column)
  )
  const update = (index: number, key: PreviewSortKey) =>
    setDraft((keys) =>
      keys.map((current, at) => (at === index ? key : current))
    )
  const move = (index: number, by: number) =>
    setDraft((keys) => {
      const target = index + by
      if (target < 0 || target >= keys.length) return keys
      const next = [...keys]
      const [moved] = next.splice(index, 1)
      if (moved) next.splice(target, 0, moved)
      return next
    })

  return (
    <Popover open={open} onOpenChange={setOpen}>
      <PopoverTrigger render={<Button size="sm" variant="outline" />}>
        <HugeiconsIcon
          icon={Sorting05Icon}
          strokeWidth={2}
          data-icon="inline-start"
        />
        Sort
        {applied.length > 0 ? (
          <Badge variant="secondary">{applied.length}</Badge>
        ) : null}
      </PopoverTrigger>
      <PopoverContent align="end" className="w-[26rem]">
        <PopoverHeader>
          <PopoverTitle>Order the server returns rows in</PopoverTitle>
          <PopoverDescription>
            Columns, not expressions: Oxyn composes and quotes this part. The
            driver completes it with a unique key when one is known.
          </PopoverDescription>
        </PopoverHeader>
        {columns === null ? (
          <div className="flex flex-col items-start gap-2 text-xs text-muted-foreground">
            <p>
              The columns of this relation have not been read yet. Load them to
              choose an order.
            </p>
            <Button
              size="sm"
              variant="outline"
              disabled={loadingColumns}
              onClick={onLoadColumns}
            >
              {loadingColumns ? <Spinner data-icon="inline-start" /> : null}
              Load columns
            </Button>
          </div>
        ) : (
          <div className="flex flex-col gap-2">
            {draft.length === 0 ? (
              <p className="text-xs text-muted-foreground">
                Unsorted: whatever order the server happens to return.
              </p>
            ) : (
              <ol
                ref={keyList}
                className="flex flex-col gap-1.5"
                aria-label="Sort keys"
              >
                {draft.map((key, index) => (
                  <li
                    key={`${index}:${key.column}`}
                    className="flex items-center gap-1"
                  >
                    <span className="w-4 text-right text-xs text-muted-foreground tabular-nums">
                      {index + 1}
                    </span>
                    <NativeSelect
                      size="sm"
                      className="min-w-0 flex-1"
                      aria-label={`Sort column ${index + 1}`}
                      value={key.column}
                      onChange={(event) =>
                        update(index, { ...key, column: event.target.value })
                      }
                    >
                      {[key.column, ...unused].map((column) => (
                        <NativeSelectOption key={column} value={column}>
                          {column}
                        </NativeSelectOption>
                      ))}
                    </NativeSelect>
                    <NativeSelect
                      size="sm"
                      aria-label={`Direction of sort column ${index + 1}`}
                      value={key.descending ? "desc" : "asc"}
                      onChange={(event) =>
                        update(index, {
                          ...key,
                          descending: event.target.value === "desc",
                        })
                      }
                    >
                      <NativeSelectOption value="asc">
                        Ascending
                      </NativeSelectOption>
                      <NativeSelectOption value="desc">
                        Descending
                      </NativeSelectOption>
                    </NativeSelect>
                    <Button
                      size="icon-xs"
                      variant="ghost"
                      aria-label={`Move sort column ${index + 1} up`}
                      data-move="up"
                      data-index={index}
                      disabled={index === 0}
                      onClick={() => {
                        follow.current = { index: index - 1, way: "up" }
                        move(index, -1)
                      }}
                    >
                      <HugeiconsIcon icon={ArrowUp01Icon} strokeWidth={2} />
                    </Button>
                    <Button
                      size="icon-xs"
                      variant="ghost"
                      aria-label={`Move sort column ${index + 1} down`}
                      data-move="down"
                      data-index={index}
                      disabled={index === draft.length - 1}
                      onClick={() => {
                        follow.current = { index: index + 1, way: "down" }
                        move(index, 1)
                      }}
                    >
                      <HugeiconsIcon icon={ArrowDown01Icon} strokeWidth={2} />
                    </Button>
                    <Button
                      size="icon-xs"
                      variant="ghost"
                      aria-label={`Remove sort column ${index + 1}`}
                      onClick={() =>
                        setDraft((keys) => keys.filter((_, at) => at !== index))
                      }
                    >
                      <HugeiconsIcon icon={Delete02Icon} strokeWidth={2} />
                    </Button>
                  </li>
                ))}
              </ol>
            )}
            <div className="flex items-center gap-2">
              <Button
                size="sm"
                variant="ghost"
                disabled={unused.length === 0 || draft.length >= MAX_SORT_KEYS}
                onClick={() => {
                  const column = unused[0]
                  if (column)
                    setDraft((keys) => [...keys, { column, descending: false }])
                }}
              >
                Add column
              </Button>
              <div className="ml-auto flex items-center gap-2">
                <Button
                  size="sm"
                  variant="outline"
                  disabled={applied.length === 0 && draft.length === 0}
                  onClick={() => {
                    setOpen(false)
                    onApply([])
                  }}
                >
                  Unsorted
                </Button>
                <Button
                  size="sm"
                  disabled={draft.length === 0}
                  onClick={() => {
                    setOpen(false)
                    onApply(draft)
                  }}
                >
                  Apply sort
                </Button>
              </div>
            </div>
          </div>
        )}
      </PopoverContent>
    </Popover>
  )
}

/**
 * The filter, sort and page controls of a relation preview (Figma `190:1618`,
 * ADR-0020, ADR-0028).
 *
 * Absent — not disabled — when the session can neither filter nor sort a
 * preview (ADR-0003). The predicate is SQL the user writes; typing reads
 * nothing, Apply or Enter does. Pages exist only over rows that came from a
 * total order, and each page is a new read.
 */
export function PreviewControls({
  canFilter,
  canSort,
  columns,
  applied,
  status,
  pagination,
  loadingColumns = false,
  onApplyPredicate,
  onApplySort,
  onPage,
  onCancel,
  onLoadColumns,
}: {
  canFilter: boolean
  canSort: boolean
  /** Column names from the catalog; `null` while the relation is undescribed. */
  columns: Array<string> | null
  /** The shape the rows on screen came from. */
  applied: PreviewShape
  status: PreviewStatus
  pagination: Pagination | null
  loadingColumns?: boolean
  onApplyPredicate: (predicate: string) => void
  onApplySort: (sort: Array<PreviewSortKey>) => void
  onPage: (forward: boolean) => void
  onCancel: () => void
  onLoadColumns: () => void
}) {
  const [draft, setDraft] = React.useState(applied.predicate ?? "")

  if (!canFilter && !canSort) return null

  const pending = canFilter && draftDiffers(draft, applied.predicate)
  const notice = previewNotice({
    status,
    pending,
    filtered: (applied.predicate ?? "").trim() !== "",
    pagination,
    applied,
  })
  const pages =
    (status === "loaded" || status === "empty") &&
    pagination?.type === "ready" &&
    (pagination.previous || pagination.next)
      ? pagination
      : null

  return (
    <div className="flex shrink-0 flex-col gap-1 border-b px-3 py-1.5">
      <div className="flex items-center gap-2">
        {canFilter ? (
          <InputGroup className="h-8 min-w-0 flex-1">
            <InputGroupAddon>
              <InputGroupText className="font-mono text-xs">
                WHERE
              </InputGroupText>
            </InputGroupAddon>
            <InputGroupInput
              value={draft}
              onChange={(event) => setDraft(event.target.value)}
              onKeyDown={(event) => {
                if (event.key === "Enter") {
                  event.preventDefault()
                  onApplyPredicate(draft)
                } else if (event.key === "Escape" && status === "loading") {
                  event.preventDefault()
                  onCancel()
                }
              }}
              placeholder="status = 'active' AND amount > 100"
              aria-label="Preview filter predicate"
              spellCheck={false}
              className="font-mono"
            />
            <InputGroupAddon align="inline-end">
              <InputGroupButton
                size="xs"
                variant={pending ? "default" : "ghost"}
                onClick={() => onApplyPredicate(draft)}
              >
                Apply
              </InputGroupButton>
            </InputGroupAddon>
          </InputGroup>
        ) : (
          <div className="flex-1" />
        )}
        {canSort ? (
          <SortComposer
            columns={columns}
            applied={applied.sort}
            loadingColumns={loadingColumns}
            onLoadColumns={onLoadColumns}
            onApply={onApplySort}
          />
        ) : null}
      </div>
      {notice || pages ? (
        // Wraps rather than squeezing the notice into a column of two words
        // when the pager sits beside it on a narrow window.
        <div className="flex min-h-7 flex-wrap items-center gap-2">
          {notice ? (
            <p
              role="status"
              className={cn(
                "min-w-48 flex-1 text-xs",
                notice.warning ? "text-warning" : "text-muted-foreground"
              )}
            >
              {notice.text}
            </p>
          ) : (
            <div className="flex-1" />
          )}
          {pages ? (
            <div className="ms-auto flex shrink-0 items-center gap-1">
              <Button
                size="sm"
                variant="outline"
                disabled={!pages.previous}
                onClick={() => onPage(false)}
              >
                <HugeiconsIcon
                  icon={ArrowLeft01Icon}
                  strokeWidth={2}
                  data-icon="inline-start"
                />
                Previous page
              </Button>
              <Button
                size="sm"
                variant="outline"
                disabled={!pages.next}
                onClick={() => onPage(true)}
              >
                Next page
                <HugeiconsIcon
                  icon={ArrowRight01Icon}
                  strokeWidth={2}
                  data-icon="inline-end"
                />
              </Button>
            </div>
          ) : null}
        </div>
      ) : null}
    </div>
  )
}
