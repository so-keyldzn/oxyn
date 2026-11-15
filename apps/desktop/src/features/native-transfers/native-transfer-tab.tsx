import * as React from "react"
import { readNativeState } from "./native-state-read"
import { useQuery, useQueryClient } from "@tanstack/react-query"
import { NativeTransferView } from "@/components/oxyn/native-transfer-view"
import type {
  NativeTransferViewState,
  NativeBusy,
} from "@/components/oxyn/native-transfer-view"
import { NativeTransferReconcile } from "@/components/oxyn/native-transfer-reconcile"
import { NativeTransferStopReview } from "@/components/oxyn/native-transfer-stop-review"
import { BackendError, backend, newCommandId } from "@/lib/ipc/client"
import { nativeTransfers, nativeSelections } from "@/lib/ipc/native-transfers"
import type {
  NativeJob,
  NativeTool,
  NativeFormat,
  NativeOperation,
  NativeFile,
  NativeReview,
} from "@/lib/ipc/native-transfers"
import type { OpenConnection } from "@/lib/ipc/types"

export interface NativeTransferHandle {
  running: () => boolean
  requestClose: () => Promise<boolean>
}
/** Retained original-owner controller. Hiding does not cancel a native job. */
export function NativeTransferTab({
  open,
  handleRef,
  onActivity,
}: {
  open: OpenConnection
  handleRef?: React.Ref<NativeTransferHandle>
  onActivity?: () => void
}) {
  const client = useQueryClient()
  const [operation, setOperation] = React.useState<NativeOperation>("export")
  const [format, setFormat] = React.useState<NativeFormat>("customArchive")
  const [tools, setTools] = React.useState<
    Partial<Record<NativeOperation, NativeTool>>
  >({})
  const [file, setFile] = React.useState<NativeFile | null>(null)
  const [review, setReview] = React.useState<NativeReview | null>(null)
  const [busy, setBusy] = React.useState<NativeBusy>(null)
  const [cancelling, setCancelling] = React.useState(false)
  const [noOwner, setNoOwner] = React.useState(true)
  const [noPrivileges, setNoPrivileges] = React.useState(true)
  const [trustedArchive, setTrustedArchive] = React.useState(false)
  const [error, setError] =
    React.useState<NativeTransferViewState["error"]>(null)
  const [notice, setNotice] = React.useState<string | null>(null)
  const [inspected, setInspected] = React.useState<NativeJob | null>(null)
  const [closeReview, setCloseReview] = React.useState(false)
  const live = React.useRef(true)
  const task = React.useRef<{ id: string; promise: Promise<unknown> } | null>(
    null
  )
  const busyRef = React.useRef<NativeBusy>(busy)
  busyRef.current = busy
  const closeRequest = React.useRef<{
    promise: Promise<boolean>
    resolve: (closed: boolean) => void
  } | null>(null)
  const current = React.useRef({ tools, file, review, operation })
  // Resource ownership changes synchronously; React may batch the matching
  // view update past a close/unmount triggered by the same completion.
  const keys = ["native-jobs", open.connection, open.session] as const
  const support = useQuery({
    queryKey: ["native-support", open.connection, open.session],
    queryFn: ({ signal }) =>
      readNativeState(signal, (id) =>
        nativeTransfers.support(id, open.connection, open.session)
      ),
  })
  const jobs = useQuery({
    queryKey: keys,
    queryFn: ({ signal }) =>
      readNativeState(signal, (id) =>
        nativeTransfers.jobs(id, open.connection, open.session)
      ),
  })
  const progress = useQuery({
    queryKey: ["native-progress", open.connection, review?.job.id ?? null],
    queryFn: () =>
      nativeSelections.progress(open.connection, review?.job.id ?? ""),
    enabled: review !== null && (busy === "starting" || busy === "running"),
    refetchInterval:
      review !== null && (busy === "starting" || busy === "running")
        ? 500
        : false,
  })
  const issue = (caught: unknown) => ({
    message: caught instanceof Error ? caught.message : String(caught),
    retryable: caught instanceof BackendError && caught.retryable,
  })
  const revoke = (grant: string | undefined) => {
    if (grant)
      void nativeSelections.revoke(newCommandId(), grant).catch(() => undefined)
  }
  React.useEffect(() => {
    live.current = true
    return () => {
      live.current = false
      const held = task.current
      if (held) void backend.cancel(held.id).catch(() => undefined)
      const selections = current.current
      for (const tool of Object.values(selections.tools)) revoke(tool.grant)
      revoke(selections.file?.grant)
      if (selections.review)
        void nativeTransfers
          .discard(
            newCommandId(),
            open.connection,
            open.session,
            selections.review.job.id
          )
          .catch(() => undefined)
      closeRequest.current?.resolve(false)
    }
  }, [])
  const activity = React.useRef(onActivity)
  activity.current = onActivity
  React.useEffect(() => {
    activity.current?.()
  }, [busy])
  const perform = async <T,>(
    stage: Exclude<NativeBusy, null>,
    work: (id: string) => Promise<T>
  ): Promise<T | undefined> => {
    if (busyRef.current !== null) return
    const id = newCommandId()
    busyRef.current = stage
    setBusy(stage)
    setError(null)
    setNotice(null)
    setCancelling(false)
    const pending = work(id)
    task.current = { id, promise: pending }
    try {
      return await pending
    } catch (caught) {
      if (live.current) setError(issue(caught))
      return undefined
    } finally {
      if (task.current.id === id) {
        task.current = null
        busyRef.current = null
        if (live.current) {
          setBusy(null)
          setCancelling(false)
        }
      }
    }
  }
  const refreshJobs = async () => {
    await client.invalidateQueries({ queryKey: keys })
  }
  const cancel = async () => {
    const active = task.current
    if (!active) return
    setCancelling(true)
    try {
      await backend.cancel(active.id)
    } catch (caught) {
      setError(issue(caught))
      setCancelling(false)
    }
  }
  const discard = async (): Promise<boolean> => {
    const prepared = current.current.review
    if (!prepared) return true
    const result = await perform("discarding", (id) =>
      nativeTransfers.discard(
        id,
        open.connection,
        open.session,
        prepared.job.id
      )
    )
    if (!result) return false
    if (live.current) {
      current.current.review = null
      current.current.file = null
      setReview(null)
      setFile(null)
    }
    await refreshJobs()
    return true
  }
  const requestClose = async (): Promise<boolean> => {
    if (closeRequest.current) return closeRequest.current.promise
    if (busyRef.current === null) return discard()
    let resolve!: (closed: boolean) => void
    const promise = new Promise<boolean>((answer) => {
      resolve = answer
    })
    closeRequest.current = { promise, resolve }
    setCloseReview(true)
    return promise
  }
  React.useImperativeHandle(handleRef, () => ({
    running: () => busyRef.current !== null,
    requestClose,
  }))
  const keep = () => {
    setCloseReview(false)
    closeRequest.current?.resolve(false)
    closeRequest.current = null
  }
  const stopAndClose = async () => {
    const active = task.current
    await cancel()
    if (active) await active.promise.catch(() => undefined)
    // The operation's finally runs before closing drops the retained controller.
    await Promise.resolve()
    if (busyRef.current !== null) return
    const discarded = await discard()
    setCloseReview(false)
    closeRequest.current?.resolve(discarded)
    closeRequest.current = null
  }
  const activeJobs = [...(jobs.data ?? [])]
  const observed = progress.data
  if (observed) {
    const index = activeJobs.findIndex((job) => job.id === observed.id)
    if (index >= 0) activeJobs[index] = observed
    else activeJobs.unshift(observed)
  }
  return (
    <>
      <NativeTransferView
        connection={open.name}
        state={{
          support: support.data ?? null,
          operation,
          format,
          tool: tools[operation] ?? null,
          file,
          review,
          jobs: activeJobs,
          historyKnown: jobs.data !== undefined,
          reading: support.isFetching || jobs.isFetching,
          busy:
            busy === "starting" &&
            observed &&
            ["running", "finalizing", "cancelling"].includes(observed.stage)
              ? "running"
              : busy,
          cancelling: cancelling || observed?.stage === "cancelling",
          noOwner,
          noPrivileges,
          trustedArchive,
          error:
            error ??
            (support.error
              ? issue(support.error)
              : jobs.error
                ? issue(jobs.error)
                : null),
          notice,
        }}
        onOperation={(next) => {
          revoke(current.current.file?.grant)
          current.current.operation = next
          current.current.file = null
          setOperation(next)
          setFormat("customArchive")
          setFile(null)
          setTrustedArchive(false)
        }}
        onFormat={(next) => {
          revoke(current.current.file?.grant)
          current.current.file = null
          setFormat(next)
          setFile(null)
        }}
        onOptions={(options) => {
          setNoOwner(options.noOwner)
          setNoPrivileges(options.noPrivileges)
        }}
        onTrust={setTrustedArchive}
        onTool={() => {
          void (async () => {
            const selected = await perform("inspecting", (id) =>
              nativeTransfers.selectTool(
                id,
                open.connection,
                open.session,
                operation
              )
            )
            if (selected && !live.current) revoke(selected.grant)
            if (selected && live.current) {
              revoke(current.current.tools[operation]?.grant)
              current.current.tools = {
                ...current.current.tools,
                [operation]: selected,
              }
              setTools(current.current.tools)
            }
          })()
        }}
        onFile={() => {
          void (async () => {
            const selected = await perform("selectingFile", () =>
              nativeTransfers.selectFile(
                open.connection,
                open.session,
                operation,
                format
              )
            )
            if (selected && !live.current) revoke(selected.grant)
            if (selected && live.current) {
              revoke(current.current.file?.grant)
              current.current.file = selected
              setTrustedArchive(false)
              setFile(selected)
            }
          })()
        }}
        onPrepare={() => {
          void (async () => {
            const tool = tools[operation]
            if (!tool || !file) return
            const prepared = await perform("preparing", (id) =>
              nativeTransfers.prepare(id, {
                connection: open.connection,
                session: open.session,
                operation,
                format,
                tool: tool.grant,
                file: file.grant,
                noOwner: operation === "restore" ? noOwner : false,
                noPrivileges: operation === "restore" ? noPrivileges : false,
              })
            )
            if (prepared && !live.current)
              void nativeTransfers
                .discard(
                  newCommandId(),
                  open.connection,
                  open.session,
                  prepared.job.id
                )
                .catch(() => undefined)
            if (prepared && live.current) {
              current.current.review = prepared
              setReview(prepared)
              await refreshJobs()
            }
          })()
        }}
        onStart={() => {
          void (async () => {
            if (!review) return
            const finished = await perform("starting", (id) =>
              nativeTransfers.start(
                id,
                open.connection,
                open.session,
                review.job.id
              )
            )
            if (live.current) {
              if (finished) {
                current.current.review = null
                current.current.file = null
                setReview(null)
                setFile(null)
                setNotice(
                  "Native observation recorded. A new transfer requires a new preparation; no operation is replayed."
                )
              }
              await refreshJobs()
            }
          })()
        }}
        onDiscard={() => {
          void discard()
        }}
        onCancel={() => {
          void cancel()
        }}
        onCancelRead={() => {
          void client.cancelQueries({
            queryKey: ["native-support", open.connection, open.session],
          })
          void client.cancelQueries({ queryKey: keys })
        }}
        onReload={() => {
          void support.refetch()
          void jobs.refetch()
        }}
        onReconcile={setInspected}
      />
      {inspected && (
        <NativeTransferReconcile
          job={inspected}
          onCancel={() => setInspected(null)}
          onInspect={() => {
            const selected = inspected
            setInspected(null)
            void (async () => {
              await perform("reconciling", (id) =>
                nativeTransfers.reconcile(
                  id,
                  open.connection,
                  open.session,
                  selected.id
                )
              )
              await refreshJobs()
            })()
          }}
        />
      )}
      {closeReview && (
        <NativeTransferStopReview
          connection={open.name}
          restoring={current.current.operation === "restore"}
          stopping={cancelling}
          onKeep={keep}
          onStop={() => {
            void stopAndClose()
          }}
        />
      )}
    </>
  )
}
