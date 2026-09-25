import * as React from "react"
import { useQuery } from "@tanstack/react-query"
import { useStore } from "@tanstack/react-store"

import { addressKey } from "@/components/oxyn/catalog-tree"
import type { PreviewStatus } from "@/components/oxyn/preview-controls"
import type { ResultState } from "@/components/oxyn/result-panel"
import {
  createPreviewStore,
  previewKey,
} from "@/features/metadata/preview-store"
import { useRefreshSignal } from "@/features/metadata/refresh-signals"
import { backend, newCommandId } from "@/lib/ipc/client"
import { executionProgress, forget, watch } from "@/lib/ipc/events"
import { PLAIN_SHAPE, PREVIEW_ROWS, metadata } from "@/lib/ipc/metadata"
import type { PreviewShape, PreviewSortKey } from "@/lib/ipc/metadata"
import { results } from "@/lib/ipc/results"
import type { CatalogAddress, OpenConnection } from "@/lib/ipc/types"

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

/** The previews of this window, kept across the object view's mounts. */
export const previews = createPreviewStore({
  read: (id, entry, shape) =>
    metadata.previewRelation(
      id,
      entry.connection,
      entry.session,
      entry.address,
      shape
    ),
  cancel: (id) => void backend.cancel(id).catch(() => undefined),
  refuse: (command) =>
    void backend.decide(command, false).catch(() => undefined),
  forgetResult: (result) =>
    void results.forgetResult(result).catch(() => undefined),
  watch,
  unwatch: forget,
  newId: newCommandId,
  now: () => Date.now(),
})

/**
 * A relation preview with its shape: the order, the predicate and the page
 * (ADR-0020, ADR-0028).
 *
 * The preview lives in [`previews`], not in the view: coming back to an object
 * shows what was read, without reading it again, until something changes on
 * the connection. Leaving mid-read cancels the read.
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
}: {
  open: OpenConnection
  address: CatalogAddress
  enabled: boolean
  visible: boolean
}) {
  const key = previewKey(open.connection, addressKey(address))
  const entry = useStore(previews.store, (all) => all[key])
  const addressRef = React.useRef(address)
  addressRef.current = address

  React.useEffect(() => {
    if (!enabled) return
    previews.attach(key, {
      connection: open.connection,
      session: open.session,
      address: addressRef.current,
      serverCancel: open.capabilities.includes("SERVER_SIDE_CANCEL"),
    })
    return () => previews.detach(key)
  }, [enabled, key, open.session])

  const state: ResultState = entry?.state ?? { status: "initial" }
  const status = previewStatus(state)
  const applied = entry?.applied ?? PLAIN_SHAPE
  const read = (shape: PreviewShape) => previews.read(key, shape)

  // First visit, or back on a preview something changed since: read once,
  // with the shape in force. Never after an error, which a new read would hide.
  React.useEffect(() => {
    if (!entry || !visible || entry.running) return
    // The shape in force: plain on a first visit, kept across a reconnection.
    if (entry.state.status === "initial" && !entry.approvalRefused)
      read(entry.applied)
    else if (entry.stale && entry.state.status !== "error") read(entry.applied)
  }, [entry?.state.status, entry?.stale, entry?.running, visible])

  useRefreshSignal(open.connection, (signal) => {
    // A DDL sends `rowsChanged` too: the catalog signal alone adds nothing.
    if (!enabled || (signal.type !== "rowsChanged" && signal.type !== "lagged"))
      return
    previews.invalidate(open.connection)
  })

  // Rows while the read streams: the result id is known from the schema on,
  // so the grid can page it before the end.
  const progress = useStore(executionProgress, (all) =>
    entry?.running ? all[entry.running] : undefined
  )
  const streaming = progress?.result ?? null
  const streamColumns = useQuery({
    queryKey: ["result-columns", streaming] as const,
    enabled: streaming !== null,
    staleTime: Number.POSITIVE_INFINITY,
    queryFn: () => (streaming ? results.resultColumns(streaming) : null),
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
      key,
      applied,
      rows,
    ] as const,
    enabled: enabled && (status === "loaded" || status === "empty"),
    queryFn: () =>
      metadata.previewPagination(open.connection, address, applied, rows),
  })

  return {
    state: live,
    status,
    applied,
    running: Boolean(entry?.running),
    cancelling: entry?.cancelling ?? false,
    startedAt: entry?.startedAt ?? null,
    approvalRefused: entry?.approvalRefused ?? false,
    pagination: pagination.data ?? null,
    refresh: () => read(applied),
    applyPredicate: (predicate: string) =>
      read({
        sort: applied.sort,
        predicate: predicate.trim() === "" ? null : predicate,
        offset: 0,
      }),
    applySort: (sort: Array<PreviewSortKey>) =>
      // The predicate in force, not the one being typed: a sort must not
      // apply a filter nobody pressed Apply for.
      read({ sort, predicate: applied.predicate, offset: 0 }),
    page: (forward: boolean) =>
      read({
        ...applied,
        offset: Math.max(
          0,
          applied.offset + (forward ? PREVIEW_ROWS : -PREVIEW_ROWS)
        ),
      }),
    cancel: () => previews.cancel(key),
  }
}
