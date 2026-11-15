import { nativeNeedsInspection, nativeOutcome } from "./native-transfer-model"
import { Badge } from "@/components/ui/badge"
import { Button } from "@/components/ui/button"
import {
  Empty,
  EmptyHeader,
  EmptyTitle,
  EmptyDescription,
} from "@/components/ui/empty"
import { Spinner } from "@/components/ui/spinner"
import type { NativeJob } from "@/lib/ipc/native-transfers"

/** Local bounded history; no archive content, process log or automatic replay. */
export function NativeTransferJobs({
  jobs,
  busy,
  onReconcile,
}: {
  jobs: Array<NativeJob>
  busy: boolean
  onReconcile: (job: NativeJob) => void
}) {
  if (jobs.length === 0)
    return (
      <Empty className="min-h-40">
        <EmptyHeader>
          <EmptyTitle>No native transfers yet</EmptyTitle>
          <EmptyDescription>
            Completed and interrupted observations stay in readable workspace
            history. A restart never resumes a restore.
          </EmptyDescription>
        </EmptyHeader>
      </Empty>
    )
  return (
    <ul aria-label="Native transfer history" className="flex flex-col gap-3">
      {jobs.map((job) => (
        <li key={job.id} className="flex flex-col gap-2 rounded-lg border p-3">
          <div className="flex flex-wrap items-center justify-between gap-2">
            <bdi className="font-medium">{job.database}</bdi>
            <Badge
              variant={nativeNeedsInspection(job) ? "destructive" : "secondary"}
            >
              {job.stage !== "finished" && (
                <Spinner aria-hidden data-icon="inline-start" />
              )}
              {nativeOutcome(job)}
            </Badge>
          </div>
          <p className="text-sm text-muted-foreground">
            {job.operation === "export" ? "Export" : "Restore"} ·{" "}
            <bdi>{job.fileName}</bdi> · {job.tool.name} {job.tool.version}
          </p>
          {job.operation === "restore" && (
            <p className="text-xs text-muted-foreground">
              Omit source ownership: {job.noOwner ? "yes" : "no"}. Omit source
              privileges: {job.noPrivileges ? "yes" : "no"}.
            </p>
          )}
          {job.bytes > 0 && (
            <p className="text-xs text-muted-foreground">
              {job.bytes.toLocaleString()} observed output bytes · no completion
              percentage is inferred
            </p>
          )}
          {job.failure && (
            <p className="text-sm">
              Category: {job.failure}. Native diagnostics can contain database
              values and are not copied into this history.
            </p>
          )}
          {job.mayHaveChanged && (
            <p className="text-sm">
              The destination may have changed. Process termination does not
              establish rollback.
            </p>
          )}
          {nativeNeedsInspection(job) && (
            <Button
              variant="outline"
              size="sm"
              className="self-start"
              disabled={busy}
              onClick={() => onReconcile(job)}
            >
              I inspected this destination…
            </Button>
          )}
          {job.reconciled && (
            <p className="text-sm text-muted-foreground">
              Human inspection recorded. The original outcome remains unchanged.
            </p>
          )}
        </li>
      ))}
    </ul>
  )
}
