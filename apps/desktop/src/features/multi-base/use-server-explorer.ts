import * as React from "react"
import { useQuery, useQueryClient } from "@tanstack/react-query"
import { backend, BackendError, newCommandId } from "@/lib/ipc/client"
import { settingsBackend } from "@/lib/ipc/settings"
import { serverProfiles } from "@/lib/ipc/server-profiles"
import type {
  ServerProfileSnapshot,
  DiscoveredDatabase,
} from "@/lib/ipc/server-profiles"
import type { OpenConnection } from "@/lib/ipc/types"
import type { BackendFailure } from "@/components/oxyn/backend-error-alert"

export function explorerFailure(error: unknown): BackendFailure {
  return {
    message: error instanceof Error ? error.message : String(error),
    retryable: error instanceof BackendError && error.retryable,
  }
}
export interface Discovery {
  state: "initial" | "loading" | "cancelling" | "ready" | "error"
  databases: Array<DiscoveredDatabase>
  error?: BackendFailure
  seed?: string
}
export function seedFor(
  group: ServerProfileSnapshot,
  opens: Array<OpenConnection>,
  active: OpenConnection | null
) {
  const members = opens.filter(
    (open) =>
      group.connections.includes(open.connection) && open.driver === "postgres"
  )
  return (
    members.find((open) => open.connection === active?.connection) ??
    members[0] ??
    null
  )
}

/** Window-owned server reads; each result stays attached to its actual seed session. */
export function useServerExplorer(
  opens: Array<OpenConnection>,
  active: OpenConnection | null
) {
  const client = useQueryClient()
  const profiles = useQuery({
    queryKey: ["server-profiles"],
    queryFn: () => serverProfiles.list(newCommandId()),
  })
  const connections = useQuery({
    queryKey: ["connections"],
    queryFn: settingsBackend.listConnections,
  })
  const drivers = useQuery({
    queryKey: ["drivers"],
    queryFn: backend.listDrivers,
  })
  const [discoveries, setDiscoveries] = React.useState<
    Record<string, Discovery>
  >({})
  const [error, setError] = React.useState<BackendFailure | null>(null)
  const [expanded, setExpanded] = React.useState<Set<string>>(() => new Set())
  const [selected, select] = React.useState<string | null>(null)
  const running = React.useRef(
    new Map<
      string,
      { command: string; seed: string; revision: number; cancelled: boolean }
    >()
  )
  const current = React.useRef({ opens, active, groups: profiles.data ?? [] })
  current.current = { opens, active, groups: profiles.data ?? [] }
  const report = (caught: unknown) => setError(explorerFailure(caught))
  const refresh = async () => {
    await Promise.all([
      client.invalidateQueries({ queryKey: ["server-profiles"] }),
      client.invalidateQueries({ queryKey: ["connections"] }),
    ])
  }
  const cancel = async (profile: string) => {
    const flight = running.current.get(profile)
    if (!flight || flight.cancelled) return
    flight.cancelled = true
    setDiscoveries((all) => ({
      ...all,
      [profile]: { state: "cancelling", databases: [], seed: flight.seed },
    }))
    await backend.cancel(flight.command)
  }
  React.useEffect(() => {
    for (const [id, flight] of running.current) {
      const group = current.current.groups.find((g) => g.profile.id === id)
      const seed = group && seedFor(group, opens, active)
      if (
        !seed ||
        seed.session !== flight.seed ||
        group.revision !== flight.revision
      )
        void cancel(id).catch(report)
    }
  }, [opens, active, profiles.data])
  React.useEffect(
    () => () => {
      for (const flight of running.current.values())
        void backend.cancel(flight.command).catch(() => undefined)
      running.current.clear()
    },
    []
  )
  const discover = async (id: string) => {
    if (running.current.has(id)) return
    const group = current.current.groups.find((g) => g.profile.id === id)
    const seed =
      group && seedFor(group, current.current.opens, current.current.active)
    if (!group || !seed) return
    const flight = {
      command: newCommandId(),
      seed: seed.session,
      revision: group.revision,
      cancelled: false,
    }
    running.current.set(id, flight)
    setDiscoveries((all) => ({
      ...all,
      [id]: { state: "loading", databases: [], seed: seed.session },
    }))
    try {
      const databases = await serverProfiles.discover(
        flight.command,
        seed.connection
      )
      if (running.current.get(id) !== flight || flight.cancelled) return
      const now = current.current.groups.find((g) => g.profile.id === id)
      if (
        !now ||
        seedFor(now, current.current.opens, current.current.active)?.session !==
          flight.seed ||
        now.revision !== flight.revision
      )
        return
      setDiscoveries((all) => ({
        ...all,
        [id]: { state: "ready", databases, seed: seed.session },
      }))
    } catch (caught) {
      if (running.current.get(id) === flight && !flight.cancelled)
        setDiscoveries((all) => ({
          ...all,
          [id]: {
            state: "error",
            databases: [],
            error: explorerFailure(caught),
            seed: seed.session,
          },
        }))
    } finally {
      if (running.current.get(id) === flight) {
        running.current.delete(id)
        if (flight.cancelled)
          setDiscoveries((all) => ({
            ...all,
            [id]: { state: "initial", databases: [] },
          }))
      }
    }
  }
  const attach = async (profile: string) => {
    const open = current.current.active
    const group = current.current.groups.find((g) => g.profile.id === profile)
    if (!open || open.driver !== "postgres" || !group) return
    const detail = await settingsBackend.connectionDetails(open.connection)
    if (
      Object.entries(group.profile.params).some(
        ([key, value]) => key !== "user" && detail.values[key] !== value
      )
    )
      throw new Error(
        "The active connection does not match this group's server defaults."
      )
    const previous =
      current.current.groups.find((g) =>
        g.connections.includes(open.connection)
      )?.profile.id ?? null
    await serverProfiles.setMembership(
      newCommandId(),
      open.connection,
      profile,
      previous
    )
    await refresh()
  }
  const create = async (name: string) => {
    const open = current.current.active
    if (!open || open.driver !== "postgres")
      throw new Error("Open a PostgreSQL connection first.")
    const details = await settingsBackend.connectionDetails(open.connection)
    const driver = drivers.data?.find((d) => d.id === "postgres")
    if (!driver) throw new Error("PostgreSQL driver metadata is unavailable.")
    const params = Object.fromEntries(
      driver.fields
        .filter(
          (f) => !f.secret && f.key !== "database" && f.key in details.values
        )
        .map((f) => [f.key, details.values[f.key] ?? ""])
    )
    const saved = await serverProfiles.save(
      newCommandId(),
      { id: newCommandId(), name, driver: "postgres", params },
      null
    )
    // If linking fails the empty group remains visible, never silently deleted.
    await refresh()
    const previous =
      current.current.groups.find((g) =>
        g.connections.includes(open.connection)
      )?.profile.id ?? null
    await serverProfiles.setMembership(
      newCommandId(),
      open.connection,
      saved.profile.id,
      previous
    )
    await refresh()
    setExpanded((all) => new Set(all).add(saved.profile.id))
  }
  return {
    profiles,
    connections,
    drivers,
    discoveries,
    expanded,
    setExpanded,
    selected,
    select,
    error,
    report,
    refresh,
    discover,
    cancel,
    attach,
    create,
  }
}
