import { BackendErrorAlert } from "./backend-error-alert"
import type { BackendFailure } from "./backend-error-alert"
import { Button } from "@/components/ui/button"
import {
  Dialog,
  DialogContent,
  DialogHeader,
  DialogTitle,
  DialogDescription,
  DialogFooter,
} from "@/components/ui/dialog"
import type { SchemaSampleLimits } from "@/lib/ipc/metadata"

/** A sample is a separate human gesture, never an automatic metadata refresh. */
export function SchemaSamplingDialog({
  connection,
  object,
  limits,
  loading = false,
  error = null,
  onRetry,
  onSample,
  onCancel,
}: {
  connection: string
  object: string
  limits: SchemaSampleLimits | null
  loading?: boolean
  error?: BackendFailure | null
  onRetry?: () => void
  onSample: () => void
  onCancel: () => void
}) {
  return (
    <Dialog
      open
      onOpenChange={(open) => {
        if (!open) onCancel()
      }}
    >
      <DialogContent>
        <DialogHeader>
          <DialogTitle>Infer field structure?</DialogTitle>
          <DialogDescription>
            Read a bounded document sample from <bdi>{object}</bdi> on{" "}
            <bdi>{connection}</bdi>. This is a data read. Only inferred field
            metadata is kept; sample values are neither shown nor sent to AI.
          </DialogDescription>
        </DialogHeader>
        {loading ? (
          <p role="status">Reading the source’s declared sampling limits…</p>
        ) : error ? (
          <BackendErrorAlert title="Sampling limits not read" error={error} />
        ) : limits ? (
          <>
            <p className="text-sm">
              At most {limits.maxDocuments.toLocaleString()} returned documents,{" "}
              {(limits.maxRetainedBytes / (1024 * 1024)).toLocaleString()} MiB
              retained for inference, and {limits.maxDurationMs / 1000} seconds
              of server execution.
            </p>
            <p className="text-sm text-muted-foreground">
              Protocol batching and the server’s query plan can read more data.
              Missing fields may exist outside the sample; inferred structure is
              not a complete schema.
            </p>
          </>
        ) : (
          <p>
            This source has not declared supported sampling bounds. Sampling is
            unavailable.
          </p>
        )}
        <DialogFooter>
          <Button autoFocus variant="outline" onClick={onCancel}>
            Cancel
          </Button>
          {error?.retryable && onRetry && (
            <Button variant="outline" onClick={onRetry}>
              Read limits again
            </Button>
          )}
          <Button
            disabled={loading || error !== null || limits === null}
            onClick={onSample}
          >
            Read sample
          </Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  )
}
