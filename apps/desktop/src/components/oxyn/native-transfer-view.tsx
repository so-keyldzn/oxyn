import type { NativeTransferViewState } from "./native-transfer-model"
import { NativeTransferForm } from "./native-transfer-form"
import { NativeTransferJobs } from "./native-transfer-jobs"
import { BackendErrorAlert } from "./backend-error-alert"
import { Button } from "@/components/ui/button"
import {
  Card,
  CardHeader,
  CardTitle,
  CardDescription,
  CardContent,
  CardFooter,
} from "@/components/ui/card"
import { Alert, AlertTitle, AlertDescription } from "@/components/ui/alert"
import {
  Empty,
  EmptyHeader,
  EmptyTitle,
  EmptyDescription,
} from "@/components/ui/empty"
import { Progress } from "@/components/ui/progress"
import { Spinner } from "@/components/ui/spinner"
import type {
  NativeOperation,
  NativeFormat,
  NativeJob,
} from "@/lib/ipc/native-transfers"

export type {
  NativeBusy,
  NativeTransferViewState,
} from "./native-transfer-model"

/** Capability-driven native controls, without filesystem/program access or side effects. */
export function NativeTransferView({
  connection,
  state,
  onOperation,
  onFormat,
  onTool,
  onFile,
  onOptions,
  onTrust,
  onPrepare,
  onStart,
  onDiscard,
  onCancel,
  onReload,
  onCancelRead,
  onReconcile,
}: {
  connection: string
  state: NativeTransferViewState
  onOperation: (operation: NativeOperation) => void
  onFormat: (format: NativeFormat) => void
  onTool: () => void
  onFile: () => void
  onOptions: (options: { noOwner: boolean; noPrivileges: boolean }) => void
  onTrust: (trusted: boolean) => void
  onPrepare: () => void
  onStart: () => void
  onDiscard: () => void
  onCancel: () => void
  onReload: () => void
  onCancelRead: () => void
  onReconcile: (job: NativeJob) => void
}) {
  const busy = state.busy !== null
  return (
    <section
      aria-label="Database transfers"
      data-focus-region="native-transfers"
      data-focus-entry="true"
      tabIndex={-1}
      className="flex h-full min-h-0 flex-col gap-4 overflow-auto p-4 [&>*]:shrink-0"
    >
      <div className="flex flex-wrap items-start justify-between gap-3">
        <div>
          <h2 className="text-base font-semibold">Database transfers</h2>
          <p className="text-sm text-muted-foreground">
            <bdi>{connection}</bdi> · Native database export and restore,
            separate from result export
          </p>
        </div>
        <div className="flex flex-wrap gap-2">
          <Button
            variant="outline"
            size="sm"
            disabled={busy || state.reading}
            onClick={onReload}
          >
            Read availability and history
          </Button>
          {state.reading && (
            <Button variant="outline" size="sm" onClick={onCancelRead}>
              Cancel availability/history reads
            </Button>
          )}
        </div>
      </div>
      {state.error && (
        <BackendErrorAlert
          title="Native transfer did not complete"
          error={state.error}
          nextStep="Check the selected source/tool/file. Inspect any uncertain destination before preparing a new restore; nothing retries automatically."
        />
      )}
      {state.notice && (
        <p role="status" className="text-sm text-muted-foreground">
          {state.notice}
        </p>
      )}
      {!state.support ? (
        <Empty>
          <EmptyHeader>
            <EmptyTitle>Read native transfer availability</EmptyTitle>
            <EmptyDescription>
              Support follows this original source, its transport and installed
              tool selections. No program starts by opening this tab.
            </EmptyDescription>
          </EmptyHeader>
        </Empty>
      ) : (
        <>
          {state.support.reason && (
            <Alert>
              <AlertTitle>
                {state.support.export
                  ? "Some operations are unavailable"
                  : "Native transfers are unavailable"}
              </AlertTitle>
              <AlertDescription>{state.support.reason}</AlertDescription>
            </Alert>
          )}
          <NativeTransferForm
            state={state}
            onOperation={onOperation}
            onFormat={onFormat}
            onTool={onTool}
            onFile={onFile}
            onOptions={onOptions}
            onTrust={onTrust}
            onPrepare={onPrepare}
          />
          {state.review && (
            <Card>
              <CardHeader>
                <CardTitle>Fixed prepared destination</CardTitle>
                <CardDescription>
                  Options, source and file are retained; changing them requires
                  a new preparation.
                </CardDescription>
              </CardHeader>
              <CardContent className="flex flex-col gap-2">
                <p className="font-mono text-sm wrap-anywhere">
                  {state.review.destination}
                </p>
                <p className="text-sm">
                  <bdi>{state.review.job.fileName}</bdi> ·{" "}
                  {state.review.job.tool.name} {state.review.job.tool.version}
                </p>
                {state.review.job.operation === "restore" && (
                  <p className="text-sm">
                    Omit source ownership:{" "}
                    {state.review.job.noOwner ? "yes" : "no"}. Omit source
                    privileges: {state.review.job.noPrivileges ? "yes" : "no"}.
                  </p>
                )}
                {state.operation === "restore" && (
                  <Alert variant="destructive">
                    <AlertTitle>
                      Restoration can change this database
                    </AlertTitle>
                    <AlertDescription>
                      The host will independently show this actual destination
                      before execution, including for a local database.
                      Cancellation or process exit does not establish rollback.
                    </AlertDescription>
                  </Alert>
                )}
              </CardContent>
              <CardFooter className="flex flex-wrap gap-2">
                <Button variant="outline" disabled={busy} onClick={onDiscard}>
                  Discard preparation
                </Button>
                <Button disabled={busy} onClick={onStart}>
                  {state.operation === "restore"
                    ? "Review destination and restore…"
                    : "Start database export"}
                </Button>
              </CardFooter>
            </Card>
          )}
        </>
      )}
      {busy && (
        <div className="flex flex-col gap-2" role="status">
          <p className="flex items-center gap-2 text-sm">
            <Spinner aria-hidden />
            {state.cancelling
              ? "Cancellation requested; waiting for process and file cleanup…"
              : state.busy === "starting"
                ? "Waiting for host confirmation or native startup…"
                : `Native operation: ${state.busy}`}
          </p>
          <Progress
            value={null}
            aria-label="Native transfer in progress; no percentage available"
          />
          {state.busy !== "selectingFile" && (
            <Button
              variant="outline"
              className="self-start"
              disabled={state.cancelling}
              onClick={onCancel}
            >
              Cancel owned operation
            </Button>
          )}
        </div>
      )}
      <div className="flex flex-col gap-2">
        <h3 className="font-semibold">Transfer observations</h3>
        {state.historyKnown || state.jobs.length > 0 ? (
          <NativeTransferJobs
            jobs={state.jobs}
            busy={busy}
            onReconcile={onReconcile}
          />
        ) : (
          <Empty>
            <EmptyHeader>
              <EmptyTitle>
                {state.reading
                  ? "Reading transfer history…"
                  : "History has not been read"}
              </EmptyTitle>
              <EmptyDescription>
                Read availability and history to inspect recorded outcomes.
                Nothing starts or resumes automatically.
              </EmptyDescription>
            </EmptyHeader>
          </Empty>
        )}
      </div>
    </section>
  )
}
