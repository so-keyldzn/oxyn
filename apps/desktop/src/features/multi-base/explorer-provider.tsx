import * as React from "react"
import { useSelector } from "@tanstack/react-store"
import { useQueryClient } from "@tanstack/react-query"
import { MultiBaseExplorer } from "@/components/oxyn/multi-base-explorer"
import type {
  ExplorerConnection,
  MultiBaseExplorerProps,
} from "@/components/oxyn/multi-base-explorer"
import { DatabaseConnectionDialog } from "@/components/oxyn/database-connection-dialog"
import { ServerProfileDialog } from "@/components/oxyn/server-profile-dialog"
import { ConnectionChangeReview } from "@/components/oxyn/connection-change-review"
import { Alert, AlertTitle, AlertDescription } from "@/components/ui/alert"
import { Button } from "@/components/ui/button"
import { backend, newCommandId } from "@/lib/ipc/client"
import { serverProfiles } from "@/lib/ipc/server-profiles"
import type { ServerProfileSnapshot } from "@/lib/ipc/server-profiles"
import type { OpenConnection } from "@/lib/ipc/types"
import type { BackendFailure } from "@/components/oxyn/backend-error-alert"
import {
  MAX_RETAINED_WORKSPACES,
  session,
  openConnection,
  showConnection,
} from "@/features/session"
import { usePendingTransactions } from "@/features/workspace/pending-transactions"
import { useGroupDisconnect } from "./use-group-disconnect"
import { requestWorkspaceDisconnect } from "@/features/workspace/disconnect-requests"
import { WorkspaceActivations } from "./activation"
import { useDatabaseOpening } from "./use-database-opening"
import {
  useServerExplorer,
  seedFor,
  explorerFailure,
} from "./use-server-explorer"

const ExplorerContext = React.createContext<MultiBaseExplorerProps | null>(null)
export function ExplorerNavigation({
  onSkipToCatalog,
}: { onSkipToCatalog?: () => void } = {}) {
  const props = React.useContext(ExplorerContext)
  return props ? (
    <MultiBaseExplorer {...props} onSkipToCatalog={onSkipToCatalog} />
  ) : null
}

/** One navigation owner per window, independent from retained workspace lifetimes. */
export function ExplorerProvider({ children }: { children: React.ReactNode }) {
  const active = useSelector(session, (state) => state.open)
  const opens = useSelector(session, (state) => state.workspaces)
  const transactions = usePendingTransactions()
  const client = useQueryClient()
  const data = useServerExplorer(opens, active)
  const groupDisconnect = useGroupDisconnect(transactions, data.report)
  const [, changed] = React.useReducer((n: number) => n + 1, 0)
  const [cleanup, setCleanup] = React.useState<OpenConnection | null>(null)
  const reportRef = React.useRef(data.report)
  reportRef.current = data.report
  const [activations] = React.useState(
    () =>
      new WorkspaceActivations({
        command: newCommandId,
        held: (id) =>
          session.state.workspaces.some((open) => open.connection === id),
        show: (id) => {
          showConnection(id)
        },
        capacity: (pending) =>
          session.state.workspaces.length + pending < MAX_RETAINED_WORKSPACES,
        reconnect: backend.reconnect,
        adopt: (open, foreground) =>
          openConnection(open, { foreground, emptyFirstConsole: true }),
        discard: async (open) => {
          try {
            await serverProfiles.discard(open)
          } catch (caught) {
            setCleanup(open)
            reportRef.current(caught)
          }
        },
        cancel: backend.cancel,
        changed,
      })
  )
  React.useEffect(() => {
    activations.mount()
    return () => activations.dispose()
  }, [activations])
  const enter = (open: OpenConnection, key: string) =>
    openConnection(open, {
      foreground: activations.foreground(key),
      emptyFirstConsole: true,
    })
  const opening = useDatabaseOpening(activations, enter, data.report)
  const [profileDialog, setProfileDialog] = React.useState<{
    group: ServerProfileSnapshot | null
    name: string
    rename?: boolean
  } | null>(null)
  const [profileBusy, setProfileBusy] = React.useState(false)
  const profileSaving = React.useRef(false)
  const [profileError, setProfileError] = React.useState<BackendFailure | null>(
    null
  )
  const run = (work: Promise<unknown>) => {
    void work.catch(data.report)
  }
  const [baseErrors, setBaseErrors] = React.useState<
    Record<string, BackendFailure>
  >({})
  const baseRun = (key: string, promise: Promise<unknown>) => {
    setBaseErrors((all) => {
      const next = { ...all }
      delete next[key]
      return next
    })
    void promise.catch((caught) =>
      setBaseErrors((all) => ({ ...all, [key]: explorerFailure(caught) }))
    )
  }
  const saved = data.connections.data ?? []
  const groups = data.profiles.data ?? []
  const grouped = new Set(groups.flatMap((group) => group.connections))
  const connection = (id: string): ExplorerConnection | null => {
    const row = saved.find((c) => c.id === id)
    if (!row) return null
    const held = opens.find((open) => open.connection === id)
    return {
      id,
      name: held?.name ?? row.name,
      location: row.location,
      driver: held?.driver ?? row.driver,
      error: baseErrors[`connection:${id}`],
      environment: held?.environment ?? row.environment,
      readOnly: held?.readOnly ?? row.readOnly,
      state:
        activations.status(`connection:${id}`) ??
        (active?.connection === id ? "active" : held ? "open" : "saved"),
      transaction:
        transactions[id] === "open"
          ? "Transaction open in a console"
          : transactions[id] === "unknown"
            ? "Transaction state unknown in a console"
            : undefined,
    }
  }
  const canGroup =
    active?.driver === "postgres" &&
    data.profiles.isSuccess &&
    data.connections.isSuccess
  const props: MultiBaseExplorerProps = {
    groups: groups.map((group) => {
      const seed = seedFor(group, opens, active)
      const read = data.discoveries[group.profile.id]
      return {
        id: group.profile.id,
        name: group.profile.name,
        expanded: data.expanded.has(group.profile.id),
        seed: seed?.name ?? null,
        members: group.connections
          .map(connection)
          .filter((c): c is ExplorerConnection => c !== null),
        discovery:
          read && (!read.seed || read.seed === seed?.session)
            ? {
                ...read,
                databases: read.databases.map((db) => ({
                  ...db,
                  state: activations.status(
                    JSON.stringify([group.profile.id, db.name])
                  ),
                  error:
                    baseErrors[JSON.stringify([group.profile.id, db.name])],
                })),
              }
            : { state: "initial", databases: [] },
      }
    }),
    ungrouped: (data.profiles.data ? saved : [])
      .filter((c) => !grouped.has(c.id))
      .map((c) => connection(c.id))
      .filter((c): c is ExplorerConnection => c !== null),
    loading: data.profiles.isPending || data.connections.isPending,
    error:
      data.error ??
      (data.profiles.error
        ? explorerFailure(data.profiles.error)
        : data.connections.error
          ? explorerFailure(data.connections.error)
          : null),
    selected: data.selected,
    canGroup,
    onSelect: data.select,
    onToggle: (id) =>
      data.setExpanded((all) => {
        const next = new Set(all)
        if (next.has(id)) next.delete(id)
        else next.add(id)
        return next
      }),
    onActivate: (id) => baseRun(`connection:${id}`, activations.activate(id)),
    onCancel: (key) =>
      run(
        key.startsWith("discovery:")
          ? data.cancel(key.slice(10))
          : activations.cancel(key)
      ),
    onDiscover: (id) => run(data.discover(id)),
    onDatabase: (id, name, fresh) => {
      const group = groups.find((g) => g.profile.id === id)
      const seed = group && seedFor(group, opens, active)
      const driver = data.drivers.data?.find((d) => d.id === "postgres")
      if (!group || !seed || !driver) {
        setBaseErrors((all) => ({
          ...all,
          [JSON.stringify([id, name])]: {
            message:
              "Open a grouped PostgreSQL connection and load its driver before opening another database.",
            retryable: false,
          },
        }))
        return
      }
      baseRun(
        JSON.stringify([id, name]),
        opening.choose(group, seed, name, driver, fresh)
      )
    },
    onCreateGroup: () => {
      setProfileError(null)
      setProfileDialog({ group: null, name: active?.name ?? "Server" })
    },
    onDisconnectGroup: (id) => {
      const group = groups.find((g) => g.profile.id === id)
      if (group) groupDisconnect.request(group)
    },
    onDisconnect: requestWorkspaceDisconnect,
    onUnlink: (id) => {
      const group = groups.find((g) => g.connections.includes(id))
      if (group)
        run(
          serverProfiles
            .setMembership(newCommandId(), id, null, group.profile.id)
            .then(data.refresh)
        )
    },
    onRenameGroup: (id) => {
      const group = groups.find((g) => g.profile.id === id)
      if (group) {
        setProfileError(null)
        setProfileDialog({ group, name: group.profile.name, rename: true })
      }
    },
    onRemoveGroup: (id) => {
      const group = groups.find((g) => g.profile.id === id)
      if (group) {
        setProfileError(null)
        setProfileDialog({ group, name: group.profile.name })
      }
    },
    onAttach: (id) => run(data.attach(id)),
    onRefresh: () => run(data.refresh()),
  }
  const saveProfile = async (name: string) => {
    if (!profileDialog || profileSaving.current) return
    profileSaving.current = true
    setProfileBusy(true)
    try {
      if (profileDialog.group && profileDialog.rename)
        await serverProfiles.save(
          newCommandId(),
          { ...profileDialog.group.profile, name },
          profileDialog.group.revision
        )
      else if (profileDialog.group)
        await serverProfiles.remove(
          newCommandId(),
          profileDialog.group.profile.id,
          profileDialog.group.revision
        )
      else await data.create(name)
      await data.refresh()
      setProfileDialog(null)
    } catch (caught) {
      setProfileError(explorerFailure(caught))
      await client.invalidateQueries({ queryKey: ["server-profiles"] })
    } finally {
      profileSaving.current = false
      setProfileBusy(false)
    }
  }
  return (
    <ExplorerContext.Provider value={props}>
      {children}
      {groupDisconnect.element}
      {(groupDisconnect.busy || groupDisconnect.result) && (
        <Alert className="fixed right-4 bottom-4 max-w-md">
          <AlertTitle>
            {groupDisconnect.busy
              ? "Reviewing group disconnection"
              : "Group disconnection result"}
          </AlertTitle>
          <AlertDescription>
            {groupDisconnect.result ??
              "Complete or cancel the current workspace review to continue."}
          </AlertDescription>
          {!groupDisconnect.busy && (
            <Button variant="outline" onClick={groupDisconnect.dismiss}>
              Dismiss
            </Button>
          )}
        </Alert>
      )}
      {cleanup && (
        <Alert className="fixed bottom-4 left-4 max-w-sm">
          <AlertTitle>Cancelled opening cleanup</AlertTitle>
          <AlertDescription>
            A cancelled opening still needs cleanup.
          </AlertDescription>
          <Button
            variant="outline"
            onClick={() =>
              run(serverProfiles.discard(cleanup).then(() => setCleanup(null)))
            }
          >
            Finish cleanup
          </Button>
        </Alert>
      )}
      {profileDialog && (
        <ServerProfileDialog
          name={profileDialog.name}
          removing={profileDialog.group !== null && !profileDialog.rename}
          renaming={profileDialog.rename}
          busy={profileBusy}
          error={profileError}
          onSave={(name) => run(saveProfile(name))}
          onClose={() => setProfileDialog(null)}
        />
      )}
      {opening.wizard && !opening.approval && (
        <DatabaseConnectionDialog
          database={opening.wizard.database}
          driver={opening.wizard.driver}
          prefill={{
            name: opening.wizard.database,
            readOnly: false,
            values: opening.wizard.values,
          }}
          candidates={opening.wizard.candidates}
          submitting={opening.wizard.submitting}
          error={opening.wizard.error}
          saved={opening.wizard.saved}
          onSubmit={(draft) => run(opening.submit(draft))}
          onChoose={(id) => run(opening.chooseSaved(id))}
          onFresh={opening.fresh}
          onClose={opening.close}
          onCancel={() => run(opening.cancel())}
        />
      )}
      <ConnectionChangeReview
        change={opening.approval}
        deciding={opening.deciding}
        onDecide={(approved) => run(opening.decide(approved))}
      />
    </ExplorerContext.Provider>
  )
}
