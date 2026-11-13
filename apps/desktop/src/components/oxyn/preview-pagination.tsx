import { HugeiconsIcon } from "@hugeicons/react"
import * as React from "react"
import { ArrowLeft01Icon, ArrowRight01Icon } from "@hugeicons/core-free-icons"

import { BackendErrorAlert } from "./backend-error-alert"
import type { BackendFailure } from "./backend-error-alert"

import { Button } from "@/components/ui/button"
import { Label } from "@/components/ui/label"
import {
  Select,
  SelectContent,
  SelectGroup,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select"
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
  error = null,
  onRetry,
  rows,
  status,
  onPage,
  onPageSize,
}: {
  applied: PreviewShape
  pagination: PageAvailability | null
  error?: BackendFailure | null
  onRetry?: () => void
  /** Only settled rows have a range; a running read has not produced a page. */
  rows: number | null
  status: PreviewStatus
  onPage: (forward: boolean) => void
  onPageSize: (size: PreviewPageSize) => void
}) {
  const pageSizeId = React.useId()
  if (pagination?.type === "notApplicable") return null
  const settled = status === "loaded" || status === "empty"
  const ready =
    !error &&
    settled &&
    (pagination?.type === "ready" || pagination?.type === "retained")
      ? pagination
      : null
  const busy = status === "loading" || status === "initial"
  const pageable = pagination?.type !== "noUniqueKey"
  const page = Math.floor(applied.offset / applied.pageSize) + 1
  const range =
    rows === null
      ? status === "loading"
        ? "Loading rows…"
        : "Rows not loaded"
      : rows === 0
        ? "No rows"
        : `Rows ${(applied.offset + 1).toLocaleString("en-US")}–${(applied.offset + rows).toLocaleString("en-US")}`
  const reason = error
    ? null
    : pagination?.type === "needsNativeOrder"
      ? "Clear the sort to browse this server's native order."
      : pagination?.type === "noUniqueKey"
        ? "Only a bounded preview is available: this source has no supported continuation or proven key."
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
      {error ? (
        <div className="w-full">
          <BackendErrorAlert
            title="Page navigation unavailable"
            error={error}
            onRetry={onRetry}
            retryLabel="Retry page navigation"
            nextStep="Resolve this error, then refresh the preview."
          />
        </div>
      ) : null}
      <span role="status" className="text-muted-foreground tabular-nums">
        {range}
        {pageable ? ` · Page ${page.toLocaleString("en-US")}` : ""}
      </span>
      {reason ? <span className="text-muted-foreground">{reason}</span> : null}
      <div className="ms-auto flex flex-wrap items-center gap-3">
        <div className="flex items-center gap-2">
          <Label htmlFor={pageSizeId}>
            {pageable ? "Rows per page" : "Preview rows"}
          </Label>
          <Select
            items={PREVIEW_PAGE_SIZES.map((size) => ({
              value: size,
              label: String(size),
            }))}
            value={applied.pageSize}
            disabled={busy}
            onValueChange={(value) => {
              const size = PreviewPageSize.safeParse(value)
              if (size.success) onPageSize(size.data)
            }}
          >
            <SelectTrigger
              id={pageSizeId}
              size="sm"
              aria-label={pageable ? "Rows per page" : "Preview rows"}
            >
              <SelectValue />
            </SelectTrigger>
            <SelectContent alignItemWithTrigger={false}>
              <SelectGroup>
                {PREVIEW_PAGE_SIZES.map((size) => (
                  <SelectItem key={size} value={size}>
                    {size}
                  </SelectItem>
                ))}
              </SelectGroup>
            </SelectContent>
          </Select>
        </div>
        {pageable ? (
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
        ) : null}
      </div>
    </div>
  )
}
