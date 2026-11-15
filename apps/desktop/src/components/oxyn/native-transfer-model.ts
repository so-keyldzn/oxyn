import type {
  NativeJob,
  NativeSupport,
  NativeOperation,
  NativeFormat,
  NativeTool,
  NativeFile,
  NativeReview,
} from "@/lib/ipc/native-transfers"
import type { BackendFailure } from "./backend-error-alert"

export function nativeOutcome(job: NativeJob): string {
  switch (job.outcome) {
    case "succeeded":
      return "Completed"
    case "refused":
      return "Refused before execution"
    case "failedBeforeStart":
      return "Failed before target execution"
    case "failed":
      return "Export failed"
    case "cancelled":
      return "Cancelled"
    case "possiblePartial":
      return "Possibly applied in part"
    case "unknown":
      return "Outcome unknown"
    case "interrupted":
      return "Interrupted; inspect the outcome"
    default:
      return job.stage === "ready" ? "Prepared; nothing started" : job.stage
  }
}
export function nativeNeedsInspection(job: NativeJob): boolean {
  return (
    job.stage === "finished" &&
    job.mayHaveChanged &&
    !job.reconciled &&
    job.outcome !== "succeeded"
  )
}

export type NativeBusy =
  | "inspecting"
  | "selectingFile"
  | "preparing"
  | "starting"
  | "running"
  | "discarding"
  | "reconciling"
  | null
export interface NativeTransferViewState {
  support: NativeSupport | null
  operation: NativeOperation
  format: NativeFormat
  tool: NativeTool | null
  file: NativeFile | null
  review: NativeReview | null
  jobs: Array<NativeJob>
  historyKnown: boolean
  reading: boolean
  busy: NativeBusy
  cancelling: boolean
  noOwner: boolean
  noPrivileges: boolean
  trustedArchive: boolean
  error: BackendFailure | null
  notice?: string | null
}
