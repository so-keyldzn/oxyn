import { HugeiconsIcon } from "@hugeicons/react"
import { Alert02Icon, Copy01Icon } from "@hugeicons/core-free-icons"

import { Alert, AlertDescription, AlertTitle } from "@/components/ui/alert"
import { Button } from "@/components/ui/button"
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog"
import { Spinner } from "@/components/ui/spinner"
import type { ValuePageView } from "@/lib/ipc/results"

export type ValuePageState =
  | { status: "loading" }
  | { status: "page"; page: ValuePageView }
  | { status: "expired" }
  | { status: "error"; message: string }

/** « Rendered bytes 0–16384 of 48213 ». Pure, so it is tested without a DOM. */
export function renderedBytes(page: ValuePageView) {
  const end = page.nextOffset ?? page.totalBytes
  return `Rendered bytes ${page.offset.toLocaleString("en-US")}–${end.toLocaleString("en-US")} of ${page.totalBytes.toLocaleString("en-US")}`
}

/**
 * `Inspect full value`: one value, in pages of at most 16 KiB cut on UTF-8
 * boundaries, read from the existing result (`InspectResultValue`).
 *
 * An absent value, an empty value and the text `NULL` stay distinct. Closing
 * while a page loads cancels it; the error state keeps a way to close and to
 * try again, and neither runs SQL (docs/UX-SPEC.md).
 */
export function ValuePageDialog({
  open,
  connectionName,
  column,
  row,
  state,
  canGoBack,
  onPrevious,
  onNext,
  onRetry,
  onCopyPage,
  onClose,
}: {
  open: boolean
  connectionName: string
  column: string
  row: number
  state: ValuePageState
  canGoBack: boolean
  onPrevious: () => void
  onNext: () => void
  onRetry: () => void
  onCopyPage: (text: string) => void
  onClose: () => void
}) {
  const page = state.status === "page" ? state.page : null

  return (
    <Dialog open={open} onOpenChange={(next) => (next ? undefined : onClose())}>
      <DialogContent className="flex max-h-[80vh] flex-col sm:max-w-3xl">
        <DialogHeader>
          <DialogTitle>Full value · Read only</DialogTitle>
          <DialogDescription className="wrap-anywhere">
            <bdi>{connectionName}</bdi> · <bdi>{column}</bdi> · row{" "}
            {(row + 1).toLocaleString("en-US")}
          </DialogDescription>
        </DialogHeader>

        <div className="min-h-40 flex-1 overflow-auto rounded-md border bg-card">
          {state.status === "loading" ? (
            <p
              role="status"
              className="flex items-center gap-2 p-3 text-sm text-muted-foreground"
            >
              <Spinner /> Loading existing value…
            </p>
          ) : state.status === "expired" ? (
            <p className="p-3 text-sm text-muted-foreground">
              This result is no longer available. Oxyn never runs the query
              again to bring a value back.
            </p>
          ) : state.status === "error" ? (
            <Alert variant="destructive" className="border-0">
              <HugeiconsIcon icon={Alert02Icon} strokeWidth={2} />
              <AlertTitle>The value could not be read</AlertTitle>
              <AlertDescription>
                <pre
                  data-selectable
                  className="font-mono text-xs whitespace-pre-wrap"
                >
                  {state.message}
                </pre>
              </AlertDescription>
            </Alert>
          ) : page?.isNull ? (
            <p className="p-3 font-mono text-sm text-null italic" data-null>
              ∅ NULL
            </p>
          ) : page?.totalBytes === 0 ? (
            <p className="p-3 text-sm text-muted-foreground">
              Empty value (0 bytes)
            </p>
          ) : (
            <pre
              data-selectable
              tabIndex={0}
              aria-label={`Value of ${column}`}
              className="p-3 font-mono text-xs break-all whitespace-pre-wrap outline-none focus-visible:ring-2 focus-visible:ring-ring"
            >
              {page?.text}
            </pre>
          )}
        </div>

        <p className="text-xs text-muted-foreground">
          {page ? `${renderedBytes(page)} · ` : ""}
          {page?.dataType ? `${page.dataType} · ` : ""}
          No SQL executed
        </p>

        <DialogFooter>
          {page && !page.isNull && page.totalBytes > 0 ? (
            <Button
              variant="outline"
              className="mr-auto"
              onClick={() => onCopyPage(page.text)}
            >
              <HugeiconsIcon
                icon={Copy01Icon}
                strokeWidth={2}
                data-icon="inline-start"
              />
              {page.offset === 0 && page.nextOffset === null
                ? "Copy value"
                : "Copy this page"}
            </Button>
          ) : null}
          {state.status === "error" ? (
            <Button variant="outline" onClick={onRetry}>
              Inspect again
            </Button>
          ) : null}
          <Button
            variant="outline"
            disabled={!canGoBack || state.status === "loading"}
            onClick={onPrevious}
          >
            Previous
          </Button>
          <Button
            variant="outline"
            disabled={page?.nextOffset == null}
            onClick={onNext}
          >
            Next
          </Button>
          <Button onClick={onClose}>
            {state.status === "loading" ? "Cancel inspection" : "Close"}
          </Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  )
}
