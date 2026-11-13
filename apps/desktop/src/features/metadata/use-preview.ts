import * as React from "react"
import { useQuery } from "@tanstack/react-query"
import { useSelector } from "@tanstack/react-store"

import { addressKey } from "@/components/oxyn/catalog-tree"
import { sameShape } from "@/components/oxyn/preview-controls"
import type { PreviewStatus } from "@/components/oxyn/preview-controls"
import type { ResultState } from "@/components/oxyn/result-panel"
import { previewKey } from "@/features/metadata/preview-store"
import { previews } from "@/features/metadata/preview-state"
import { useRefreshSignal } from "@/features/metadata/refresh-signals"
import { BackendError } from "@/lib/ipc/client"
import { executionProgress } from "@/lib/ipc/events"
import { PLAIN_SHAPE, metadata } from "@/lib/ipc/metadata"
import type {
  PreviewPageSize,
  PreviewShape,
  PreviewSortKey,
} from "@/lib/ipc/metadata"
import { results } from "@/lib/ipc/results"
import type { CatalogAddress, OpenConnection } from "@/lib/ipc/types"

export { previews } from "@/features/metadata/preview-state"

/** The preview area's state, from the result panel's. Pure, so it is tested. */
export function previewStatus(state: ResultState): PreviewStatus {
  switch (state.status) {
    case "running":
      return "loading"
    case "populated":
      return state.rows === 0 && state.complete ? "empty" : "loaded"
    case "error":
      return "failed"
    default:
      return "initial"
  }
}

/**
 * A relation preview with its shape: the order, the predicate and the page
 * (ADR-0020, ADR-0028).
 *
 * The preview lives in [`previews`], not in the view: coming back to an object
 * shows what was read, without reading it again, until something changes on
 * the connection. SQL reads keep their owner across navigation, including
 * while pending. Other automatic reads stop when hidden and restart on return.
 *
 * Reads again by itself only after a write or a DDL succeeded on this
 * connection, only while `visible`, never after its own error, and with the
 * shape in force (ADR-0022).
 */
export function usePreview({
  open,
  address,
  enabled,
  visible,
  described,
  ready = true,
  rememberShape = true,
}: {
  open: OpenConnection
  address: CatalogAddress
  enabled: boolean
  visible: boolean
  /**
   * When the relation's description was last read, `null` before: the
   * backend reads the key it pages by from that description (ADR-0068), so
   * the pagination is asked again when it arrives.
   */
  described: string | null
  /** Initial metadata is read before an exclusive source is reserved. */
  ready?: boolean
  rememberShape?: boolean
}) {
  const key = previewKey(open.connection, addressKey(address), open.session)
  const entry = useSelector(previews.store, (all) => all[key])
  const addressRef = React.useRef(address)
  addressRef.current = address
  const preserveOwner = open.capabilities.includes("SQL")

  React.useEffect(() => {
    if (!enabled) return
    previews.attach(key, {
      connection: open.connection,
      session: open.session,
      address: addressRef.current,
      serverCancel: open.capabilities.includes("SERVER_SIDE_CANCEL"),
      preserveOwner,
      shapeGroup: rememberShape
        ? previewKey(open.connection, addressKey(addressRef.current))
        : undefined,
    })
    return () => previews.detach(key)
  }, [enabled, key, open.session, rememberShape, preserveOwner])

  const state: ResultState = entry?.state ?? { status: "initial" }
  const status = previewStatus(state)
  const applied = entry?.applied ?? PLAIN_SHAPE
  const read = (shape: PreviewShape, automatic = false) =>
    previews.read(
      key,
      automatic
        ? {
            ...shape,
            continuation: null,
            offset: shape.continuation ? 0 : shape.offset,
          }
        : shape,
      automatic
    )

  // Only reproducible, non-SQL reads may stop merely because they became hidden.
  React.useEffect(() => {
    if (!visible) previews.pause(key)
  }, [visible, key])

  // First visit, or back on a preview something changed since: read once,
  // with the shape in force. Never after an error, which a new read would hide.
  React.useEffect(() => {
    if (!ready || !entry || !visible || entry.running) return
    // The shape in force: plain on a first visit, kept across a reconnection.
    if (entry.state.status === "initial" && !entry.approvalRefused)
      read(entry.applied, true)
    else if (entry.stale && entry.state.status !== "error")
      read(entry.applied, true)
  }, [ready, entry?.state.status, entry?.stale, entry?.running, visible])

  useRefreshSignal(open.connection, (signal) => {
    // A DDL sends `rowsChanged` too: the catalog signal alone adds nothing.
    if (!enabled || (signal.type !== "rowsChanged" && signal.type !== "lagged"))
      return
    previews.invalidate(open.connection)
  })

  // Rows while the read streams: the result id is known from the schema on,
  // so the grid can page it before the end.
  const progress = useSelector(executionProgress, (all) =>
    entry?.running ? all[entry.running] : undefined
  )
  const streaming = progress?.result ?? null
  const streamColumns = useQuery({
    queryKey: ["result-columns", open.connection, streaming] as const,
    enabled: streaming !== null,
    staleTime: Number.POSITIVE_INFINITY,
    queryFn: () =>
      streaming ? results.resultColumns(open.connection, streaming) : null,
  })
  const live = React.useMemo<ResultState>(
    () =>
      state.status === "running"
        ? {
            ...state,
            rows: progress?.rows ?? state.rows,
            result: streaming,
            columns: streamColumns.data ?? null,
          }
        : state,
    [state, progress?.rows, streaming, streamColumns.data]
  )

  const rows = state.status === "populated" ? state.rows : 0
  const pagination = useQuery({
    queryKey: [
      "preview-pagination",
      open.connection,
      open.session,
      state.status === "populated" ? state.result : null,
      key,
      applied,
      rows,
      described,
    ] as const,
    enabled: enabled && (status === "loaded" || status === "empty"),
    queryFn: () =>
      metadata.previewPagination(
        open.connection,
        address,
        applied,
        rows,
        open.session,
        state.status === "populated" ? state.result : null
      ),
  })

  return {
    state: live,
    status,
    applied,
    reshaped: entry ? !sameShape(entry.requested, entry.applied) : false,
    running: Boolean(entry?.running),
    cancelling: entry?.cancelling ?? false,
    startedAt: entry?.startedAt ?? null,
    approvalRefused: entry?.approvalRefused ?? false,
    pagination: pagination.isError ? null : (pagination.data ?? null),
    paginationError: pagination.error
      ? {
          message: pagination.error.message,
          retryable:
            pagination.error instanceof BackendError &&
            pagination.error.retryable,
        }
      : null,
    retryPagination: () => void pagination.refetch(),
    refresh: () =>
      read({
        ...applied,
        continuation: null,
        offset:
          pagination.data?.type === "retained" || applied.continuation
            ? 0
            : applied.offset,
      }),
    applyPredicate: (predicate: string) =>
      read({
        ...applied,
        predicate: predicate.trim() === "" ? null : predicate,
        offset: 0,
        continuation: null,
      }),
    applySort: (sort: Array<PreviewSortKey>) =>
      // The predicate in force, not the one being typed: a sort must not
      // apply a filter nobody pressed Apply for.
      read({ ...applied, sort, offset: 0, continuation: null }),
    resizePage: (pageSize: PreviewPageSize) =>
      read({ ...applied, pageSize, offset: 0, continuation: null }),
    page: (forward: boolean) =>
      read({
        ...applied,
        continuation:
          pagination.data?.type === "retained" && state.status === "populated"
            ? state.result
            : null,
        offset: Math.max(
          0,
          applied.offset + (forward ? applied.pageSize : -applied.pageSize)
        ),
      }),
    cancel: () => previews.cancel(key),
  }
}
