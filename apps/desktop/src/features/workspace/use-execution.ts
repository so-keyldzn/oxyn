import * as React from "react"
import { useStore } from "@tanstack/react-store"

import type { PendingApproval } from "@/components/oxyn/approval-dialog"
import type { ResultState } from "@/components/oxyn/result-panel"
import type { ExecutionSummary } from "@/components/oxyn/status-bar"
import { BackendError, backend, newCommandId } from "@/lib/ipc/client"
import { executionProgress, forget, watch } from "@/lib/ipc/events"
import { results } from "@/lib/ipc/results"
import type { CommandOutcome, ResultColumn } from "@/lib/ipc/types"

/** Maps a backend outcome to what the panel draws. Pure, so it is tested. */
export function stateFromOutcome(outcome: CommandOutcome): {
  state: ResultState
  approval: PendingApproval | null
  summary: ExecutionSummary
} {
  switch (outcome.type) {
    case "executed":
      return {
        state: {
          status: "populated",
          result: outcome.result,
          columns: outcome.columns,
          rows: outcome.rows,
          complete: outcome.complete,
          truncated: outcome.truncated,
          cancelled: outcome.cancelled,
          elapsedMs: outcome.elapsedMs,
        },
        approval: null,
        summary: outcome.cancelled
          ? { status: "cancelled" }
          : {
              status: "done",
              rows: outcome.rows,
              elapsedMs: outcome.elapsedMs,
            },
      }
    case "needsApproval":
      return {
        state: { status: "initial" },
        approval: {
          command: outcome.command,
          reason: outcome.reason,
          preview: outcome.preview,
        },
        summary: { status: "idle" },
      }
    case "denied":
      return {
        state: { status: "error", message: outcome.reason, retryable: false },
        approval: null,
        summary: { status: "failed" },
      }
    case "cancelled":
      return {
        state: { status: "empty", message: "The statement was cancelled." },
        approval: null,
        summary: { status: "cancelled" },
      }
    default:
      return {
        state: { status: "empty", message: "The command completed." },
        approval: null,
        summary: { status: "idle" },
      }
  }
}

/**
 * One cancellable execution slot: the SQL console has one, the object preview
 * another. Starting a new run in a slot cancels the previous one and ignores
 * its late answer (docs/UX-SPEC.md, « Données d'une table sélectionnée »).
 */
export function useExecution({ serverCancel }: { serverCancel: boolean }) {
  const [state, setState] = React.useState<ResultState>({ status: "initial" })
  const [summary, setSummary] = React.useState<ExecutionSummary>({
    status: "idle",
  })
  const [approval, setApproval] = React.useState<PendingApproval | null>(null)
  const [deciding, setDeciding] = React.useState(false)
  const current = React.useRef<string | null>(null)
  const heldResult = React.useRef<string | null>(null)
  const [running, setRunning] = React.useState<string | null>(null)
  /** Stop was pressed and the answer has not arrived: said, not guessed. */
  const [cancelling, setCancelling] = React.useState(false)
  /** When the current run started, for a live elapsed time. */
  const [startedAt, setStartedAt] = React.useState<number | null>(null)
  const pendingApproval = React.useRef<string | null>(null)

  const progress = useStore(executionProgress, (all) =>
    running ? all[running] : undefined
  )
  // The result is addressable from `schemaReady` on, before the last batch:
  // its columns are read once so the grid fills while the stream runs
  // (docs/UX-SPEC.md, « États d'une vue »).
  const streaming = running ? (progress?.result ?? null) : null
  const [streamed, setStreamed] = React.useState<{
    result: string
    columns: Array<ResultColumn>
  } | null>(null)
  React.useEffect(() => {
    if (streaming === null || streamed?.result === streaming) return
    let left = false
    void results
      .resultColumns(streaming)
      .then((columns) => {
        // `null`: the result expired before its columns were read. The
        // outcome still carries them, so nothing is said here.
        if (!left && columns) setStreamed({ result: streaming, columns })
      })
      .catch(() => undefined)
    return () => {
      left = true
    }
  }, [streaming, streamed])

  const release = () => {
    if (heldResult.current) {
      void backend.forgetResult(heldResult.current).catch(() => undefined)
      heldResult.current = null
    }
  }

  const settle = (id: string, outcome: CommandOutcome) => {
    if (current.current !== id) {
      // A late answer from a run the user replaced: never shown, and a held
      // approval is refused rather than left pending.
      if (outcome.type === "needsApproval")
        void backend.decide(outcome.command, false)
      if (outcome.type === "executed") void backend.forgetResult(outcome.result)
      return
    }
    const next = stateFromOutcome(outcome)
    if (outcome.type === "executed") heldResult.current = outcome.result
    pendingApproval.current =
      outcome.type === "needsApproval" ? outcome.command : null
    setState(next.state)
    setSummary(next.summary)
    setApproval(next.approval)
    setRunning(null)
    setCancelling(false)
    setStartedAt(null)
    forget(id)
  }

  const fail = (id: string, error: unknown) => {
    if (current.current !== id) return
    const message = error instanceof Error ? error.message : String(error)
    const retryable = error instanceof BackendError ? error.retryable : false
    setState({ status: "error", message, retryable })
    setSummary({ status: "failed" })
    setRunning(null)
    setCancelling(false)
    setStartedAt(null)
    forget(id)
  }

  const start = (run: (id: string) => Promise<CommandOutcome>) => {
    if (current.current) void backend.cancel(current.current)
    release()
    const id = newCommandId()
    current.current = id
    watch(id)
    setRunning(id)
    setCancelling(false)
    setStartedAt(Date.now())
    setApproval(null)
    setState({ status: "running", rows: 0, serverCancel })
    setSummary({ status: "running", rows: 0 })
    run(id).then(
      (outcome) => settle(id, outcome),
      (error: unknown) => fail(id, error)
    )
  }

  const cancel = () => {
    if (!running || cancelling) return
    setCancelling(true)
    void backend.cancel(running)
  }

  const decide = (approved: boolean) => {
    // A double click must not send the decision twice.
    if (!approval || deciding) return
    const command = approval.command
    pendingApproval.current = null
    // The decision is tracked under the pending command's own id: that is the
    // token the backend registers, and what Cancel must reach.
    current.current = command
    setDeciding(true)
    if (approved) {
      watch(command)
      setRunning(command)
      setStartedAt(Date.now())
      setState({ status: "running", rows: 0, serverCancel })
      setSummary({ status: "running", rows: 0 })
    }
    backend.decide(command, approved).then(
      (outcome) => {
        setDeciding(false)
        if (!approved) {
          // A refusal is the user's choice, not a failure to report.
          current.current = null
          setApproval(null)
          setState({ status: "initial" })
          setSummary({ status: "idle" })
          return
        }
        settle(command, outcome)
      },
      (error: unknown) => {
        setDeciding(false)
        setApproval(null)
        fail(command, error)
      }
    )
  }

  const reset = () => {
    if (current.current) void backend.cancel(current.current)
    if (pendingApproval.current)
      void backend.decide(pendingApproval.current, false).catch(() => undefined)
    pendingApproval.current = null
    current.current = null
    release()
    setCancelling(false)
    setStartedAt(null)
    setRunning(null)
    setApproval(null)
    setState({ status: "initial" })
    setSummary({ status: "idle" })
  }

  // Unmounting stops the work, not just the display: the running command is
  // cancelled at the server, a held approval is refused, and a late answer
  // lands in `settle` with no current id — which forgets its result.
  React.useEffect(
    () => () => {
      if (current.current) void backend.cancel(current.current)
      if (pendingApproval.current)
        void backend
          .decide(pendingApproval.current, false)
          .catch(() => undefined)
      pendingApproval.current = null
      current.current = null
      release()
    },
    []
  )

  // Memoized on the row count: a parent that reacts to the summary would
  // otherwise receive a new object on every render and loop.
  const liveRows = progress?.rows
  const liveColumns = streamed?.result === streaming ? streamed.columns : null
  const liveState = React.useMemo<ResultState>(
    () =>
      state.status === "running"
        ? {
            ...state,
            rows: liveRows ?? state.rows,
            result: streaming,
            columns: liveColumns,
          }
        : state,
    [state, liveRows, streaming, liveColumns]
  )
  // Cancelling is said as soon as Stop is pressed, before the server confirms
  // (UX-SPEC « Annulation »): the status bar announces it once.
  const liveSummary = React.useMemo<ExecutionSummary>(() => {
    if (summary.status !== "running") return summary
    const rows = liveRows ?? summary.rows
    return cancelling
      ? { status: "cancelling", rows }
      : liveRows === undefined
        ? summary
        : { status: "running", rows }
  }, [summary, liveRows, cancelling])

  return {
    state: liveState,
    summary: liveSummary,
    approval,
    deciding,
    running: running !== null,
    cancelling,
    startedAt,
    start,
    cancel,
    decide,
    reset,
  }
}
