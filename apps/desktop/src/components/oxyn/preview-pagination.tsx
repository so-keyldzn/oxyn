import { HugeiconsIcon } from "@hugeicons/react"
import { ArrowLeft01Icon, ArrowRight01Icon } from "@hugeicons/core-free-icons"

import { Button } from "@/components/ui/button"
import { NativeSelect, NativeSelectOption } from "@/components/ui/native-select"
import {
  Pagination,
  PaginationContent,
  PaginationItem,
} from "@/components/ui/pagination"
import { PREVIEW_PAGE_SIZES, PreviewPageSize } from "@/lib/ipc/metadata"
import type {
  Pagination as PageAvailability,
  PreviewShape,
} from "@/lib/ipc/metadata"
import type { PreviewStatus } from "./preview-controls"

/** Page navigation stays outside the scrolling result, even on a short page. */
export function PreviewPagination({
  applied,
  pagination,
  rows,
  status,
  onPage,
  onPageSize,
}: {
  applied: PreviewShape
  pagination: PageAvailability | null
  /** Only settled rows have a range; a running read has not produced a page. */
  rows: number | null
  status: PreviewStatus
  onPage: (forward: boolean) => void
  onPageSize: (size: PreviewPageSize) => void
}) {
  const settled = status === "loaded" || status === "empty"
  const ready =
    settled && (pagination?.type === "ready" || pagination?.type === "retained")
      ? pagination
      : null
  const busy = status === "loading" || status === "initial"
  const page = Math.floor(applied.offset / applied.pageSize) + 1
  const range =
    rows === null
      ? status === "loading"
        ? "Loading rows…"
        : "Rows not loaded"
      : rows === 0
        ? "No rows"
        : `Rows ${(applied.offset + 1).toLocaleString("en-US")}–${(applied.offset + rows).toLocaleString("en-US")}`
  const reason =
    pagination?.type === "needsNativeOrder"
      ? "Clear the sort to browse this server's native order."
      : pagination?.type === "noUniqueKey"
        ? "No unique key is known for this relation."
        : pagination?.type === "needsOrder"
          ? "Sort the preview to browse pages."
          : pagination?.type === "unknown" || !pagination
            ? "Page navigation waits for the relation's description."
            : pagination.type === "retained"
              ? "Pages follow one read. Refresh starts a new read."
              : null

  return (
    <div
      className="flex shrink-0 flex-wrap items-center gap-x-4 gap-y-2 border-t bg-card px-3 py-2 text-xs"
      data-slot="preview-pagination"
    >
      <span role="status" className="text-muted-foreground tabular-nums">
        {range} · Page {page.toLocaleString("en-US")}
      </span>
      {reason ? <span className="text-muted-foreground">{reason}</span> : null}
      <div className="ms-auto flex flex-wrap items-center gap-3">
        <label className="flex items-center gap-2">
          Rows per page
          <NativeSelect
            size="sm"
            value={applied.pageSize}
            disabled={busy}
            onChange={(event) => {
              const size = PreviewPageSize.safeParse(Number(event.target.value))
              if (size.success) onPageSize(size.data)
            }}
          >
            {PREVIEW_PAGE_SIZES.map((size) => (
              <NativeSelectOption key={size} value={size}>
                {size}
              </NativeSelectOption>
            ))}
          </NativeSelect>
        </label>
        <Pagination aria-label="Preview pages" className="mx-0 w-auto">
          <PaginationContent>
            <PaginationItem>
              <Button
                size="sm"
                variant="outline"
                disabled={!ready?.previous}
                onClick={() => onPage(false)}
              >
                <HugeiconsIcon
                  icon={ArrowLeft01Icon}
                  strokeWidth={2}
                  data-icon="inline-start"
                />
                Previous page
              </Button>
            </PaginationItem>
            <PaginationItem>
              <Button
                size="sm"
                variant="outline"
                disabled={!ready?.next}
                onClick={() => onPage(true)}
              >
                Next page
                <HugeiconsIcon
                  icon={ArrowRight01Icon}
                  strokeWidth={2}
                  data-icon="inline-end"
                />
              </Button>
            </PaginationItem>
          </PaginationContent>
        </Pagination>
      </div>
    </div>
  )
}
