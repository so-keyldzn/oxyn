import * as React from "react"
import { useQuery, useQueryClient } from "@tanstack/react-query"
import { MetadataLoadingDialog } from "@/components/oxyn/metadata-loading-dialog"
import type { BackendFailure } from "@/components/oxyn/backend-error-alert"
import { Button } from "@/components/ui/button"
import { loadingScopes } from "./loading-scopes"
import { hasCapability } from "@/features/session"
import { backend, BackendError, newCommandId } from "@/lib/ipc/client"
import { metadataLoading } from "@/lib/ipc/metadata-loading"
import type {
  MetadataLoadingPolicy,
  MetadataPreloadProgress,
} from "@/lib/ipc/metadata-loading"
import type { CatalogNode, OpenConnection } from "@/lib/ipc/types"

const failure = (error: unknown): BackendFailure => ({
  message: error instanceof Error ? error.message : String(error),
  retryable: error instanceof BackendError && error.retryable,
})

/** One dialog owns its preload command, including cancellation on source removal. */
export function MetadataLoadingSettings({
  open,
  nodes,
}: {
  open: OpenConnection
  nodes: Array<CatalogNode> | undefined
}) {
  const [visible, setVisible] = React.useState(false)
  return (
    <>
      <Button
        variant="ghost"
        size="sm"
        className="mx-2 group-data-[collapsible=icon]:hidden"
        onClick={() => setVisible(true)}
      >
        Metadata loading…
      </Button>
      {visible && (
        <LoadingSettings
          key={JSON.stringify([open.connection, open.session])}
          open={open}
          nodes={nodes}
          onClose={() => setVisible(false)}
        />
      )}
    </>
  )
}

function LoadingSettings({
  open,
  nodes,
  onClose,
}: {
  open: OpenConnection
  nodes: Array<CatalogNode> | undefined
  onClose: () => void
}) {
  const client = useQueryClient()
  const key = ["metadata-policy", open.connection]
  const saved = useQuery({
    queryKey: key,
    queryFn: () => metadataLoading.read(newCommandId(), open.connection),
    staleTime: 0,
  })
  const [draft, setDraft] = React.useState<MetadataLoadingPolicy | null>(null)
  const [saving, setSaving] = React.useState(false)
  const [error, setError] = React.useState<BackendFailure | null>(null)
  const [saveFailed, setSaveFailed] = React.useState(false)
  const [scopeKey, setScopeKey] = React.useState<string | null>(null)
  const [job, setJob] = React.useState<string | null>(null)
  const [cancelling, setCancelling] = React.useState(false)
  const [report, setReport] = React.useState<MetadataPreloadProgress | null>(
    null
  )
  const owner = React.useRef<string | null>(null)
  const live = React.useRef(true)
  const scopes = loadingScopes(
    nodes ?? [],
    hasCapability(open, "SCHEMAS"),
    nodes !== undefined
  )
  const selectedKey = scopes.some((scope) => scope.key === scopeKey)
    ? scopeKey
    : (scopes[0]?.key ?? null)
  const progress = useQuery({
    queryKey: ["metadata-preload", job],
    queryFn: () => metadataLoading.progress(job!),
    enabled: job !== null,
    refetchInterval: (query) => (job && !query.state.error ? 250 : false),
  })
  React.useEffect(() => {
    live.current = true
    return () => {
      live.current = false
      const id = owner.current
      owner.current = null
      if (id) void backend.cancel(id).catch(() => undefined) // Window teardown also cancels owned commands.
    }
  }, [])
  const reload = async () => {
    setError(null)
    const result = await saved.refetch()
    if (!live.current) return
    if (result.isError) setError(failure(result.error))
    else {
      setDraft(null)
      setSaveFailed(false)
      setReport(null)
    }
  }
  const save = async () => {
    if (!saved.data || saving || saveFailed) return
    setSaving(true)
    setError(null)
    try {
      const snapshot = await metadataLoading.save(
        newCommandId(),
        open.connection,
        draft ?? saved.data.policy,
        saved.data.revision
      )
      client.setQueryData(key, snapshot)
      if (live.current) {
        setDraft(null)
        if (!owner.current) setReport(null)
      }
    } catch (caught) {
      if (live.current) {
        setError(failure(caught))
        setSaveFailed(true)
      }
    } finally {
      if (live.current) setSaving(false)
    }
  }
  const preload = async (continuation: boolean) => {
    const selected = scopes.find((scope) => scope.key === selectedKey)
    if (!selected || !saved.data || owner.current || saving || saveFailed)
      return
    const id = newCommandId()
    owner.current = id
    setJob(id)
    setReport(null)
    setError(null)
    setCancelling(false)
    try {
      const result = await metadataLoading.preload(id, {
        connection: open.connection,
        session: open.session,
        scope: selected.scope,
        expectedRevision: saved.data.revision,
        refresh: false,
        after: continuation ? (report?.next ?? null) : null,
      })
      if (owner.current === id) setReport(result)
    } catch (caught) {
      if (owner.current === id) setError(failure(caught))
    } finally {
      await Promise.all([
        client.invalidateQueries({
          queryKey: ["catalog", open.connection, open.session],
        }),
        client.invalidateQueries({
          queryKey: ["relation-facets", open.connection, open.session],
        }),
        client.invalidateQueries({
          queryKey: ["catalog-search", open.connection],
        }),
      ])
      if (owner.current === id) {
        owner.current = null
        setJob(null)
        setCancelling(false)
      }
    }
  }
  const cancel = async () => {
    const id = owner.current
    if (!id || cancelling) return
    setCancelling(true)
    try {
      await backend.cancel(id)
    } catch (caught) {
      if (owner.current === id) {
        setError(failure(caught))
        setCancelling(false)
      }
    }
  }
  return (
    <MetadataLoadingDialog
      connection={open.name}
      policy={draft ?? saved.data?.policy ?? null}
      scopes={scopes}
      scopeKey={selectedKey}
      saving={saving}
      needsReload={saveFailed}
      dirty={
        draft !== null &&
        JSON.stringify(draft) !== JSON.stringify(saved.data?.policy)
      }
      running={job !== null}
      cancelling={cancelling}
      report={
        report && saved.data && report.policyRevision !== saved.data.revision
          ? { ...report, next: null }
          : (report ?? (job ? (progress.data ?? null) : null))
      }
      error={
        error ??
        (saved.error
          ? failure(saved.error)
          : progress.error
            ? failure(progress.error)
            : null)
      }
      onPolicy={setDraft}
      onScope={(scope) => {
        setScopeKey(scope)
        setReport(null)
      }}
      onSave={() => void save()}
      onReload={() => void reload()}
      onPreload={(next) => void preload(next)}
      onCancel={() => void cancel()}
      onClose={onClose}
    />
  )
}
