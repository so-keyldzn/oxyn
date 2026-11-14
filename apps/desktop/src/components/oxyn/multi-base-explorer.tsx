import * as React from "react"
import { Button } from "@/components/ui/button"
import { Badge } from "@/components/ui/badge"
import { Spinner } from "@/components/ui/spinner"
import { Separator } from "@/components/ui/separator"
import {
  Empty,
  EmptyHeader,
  EmptyTitle,
  EmptyDescription,
} from "@/components/ui/empty"
import { BackendErrorAlert } from "./backend-error-alert"
import type { BackendFailure } from "./backend-error-alert"
import { EnvironmentBadge } from "./environment-badge"
import type { Environment } from "@/lib/ipc/types"

export interface ExplorerConnection {
  id: string
  name: string
  location: string | null
  environment: Environment
  readOnly: boolean
  state: "active" | "open" | "saved" | "opening" | "cancelling"
  error?: BackendFailure
  transaction?: string
}
export interface ExplorerGroup {
  id: string
  name: string
  expanded: boolean
  seed: string | null
  members: Array<ExplorerConnection>
  discovery: {
    state: "initial" | "loading" | "cancelling" | "ready" | "error"
    databases: Array<{
      name: string
      can_connect: boolean
      state?: "opening" | "cancelling" | null
      error?: BackendFailure
    }>
    error?: BackendFailure
  }
}
export interface MultiBaseExplorerProps {
  groups: Array<ExplorerGroup>
  ungrouped: Array<ExplorerConnection>
  selected: string | null
  loading: boolean
  error: BackendFailure | null
  canGroup: boolean
  onSelect: (key: string) => void
  onToggle: (group: string) => void
  onActivate: (connection: string) => void
  onCancel: (key: string) => void
  onDiscover: (group: string) => void
  onDatabase: (group: string, name: string, fresh: boolean) => void
  onCreateGroup: () => void
  onRenameGroup: (group: string) => void
  onDisconnectGroup: (group: string) => void
  onDisconnect: (connection: string) => void
  onUnlink: (connection: string) => void
  onRemoveGroup: (group: string) => void
  onAttach: (group: string) => void
  onRefresh: () => void
}

/** Navigation selection never changes the active workspace or its protections. */
export function MultiBaseExplorer(p: MultiBaseExplorerProps) {
  const prefix = React.useId()
  const row = (connection: ExplorerConnection, grouped = false) => {
    const key = `connection:${connection.id}`
    return (
      <li key={connection.id} className="flex min-w-0 flex-col gap-1">
        <Button
          variant={p.selected === key ? "secondary" : "ghost"}
          className="h-auto w-full justify-start whitespace-normal"
          aria-pressed={p.selected === key}
          onClick={() => p.onSelect(key)}
          onDoubleClick={() => p.onActivate(connection.id)}
          onKeyDown={(e) => {
            if (e.key === "Enter") {
              e.preventDefault()
              p.onActivate(connection.id)
            }
          }}
        >
          <span className="flex min-w-0 flex-col items-start gap-1">
            <bdi>{connection.name}</bdi>
            <span className="text-xs text-muted-foreground">
              Saved destination: <bdi>{connection.location}</bdi>
            </span>
            <span className="flex flex-wrap gap-1">
              <EnvironmentBadge environment={connection.environment} />
              <Badge variant="outline">
                {connection.state === "active"
                  ? "Active"
                  : connection.state === "open"
                    ? "Open"
                    : connection.state === "cancelling"
                      ? "Cancelling"
                      : connection.state === "opening"
                        ? "Opening"
                        : "Saved"}
              </Badge>
              {connection.readOnly && (
                <Badge variant="outline">Read only</Badge>
              )}
            </span>
            {connection.transaction && (
              <span className="text-xs">{connection.transaction}</span>
            )}
          </span>
        </Button>
        {connection.state === "opening" || connection.state === "cancelling" ? (
          <Button
            variant="ghost"
            size="xs"
            disabled={connection.state === "cancelling"}
            onClick={() => p.onCancel(key)}
          >
            Cancel opening
          </Button>
        ) : (
          p.selected === key && (
            <Button
              variant="outline"
              size="xs"
              onClick={() => p.onActivate(connection.id)}
            >
              Open workspace
            </Button>
          )
        )}
        {p.selected === key && (
          <div className="flex flex-wrap gap-1">
            {(connection.state === "active" || connection.state === "open") && (
              <Button
                size="xs"
                variant="ghost"
                onClick={() => p.onDisconnect(connection.id)}
              >
                Disconnect
              </Button>
            )}
            {grouped && (
              <Button
                size="xs"
                variant="ghost"
                onClick={() => p.onUnlink(connection.id)}
              >
                Ungroup
              </Button>
            )}
          </div>
        )}
        {connection.error && (
          <BackendErrorAlert
            title="Connection not opened"
            error={connection.error}
          />
        )}
      </li>
    )
  }
  return (
    <section
      aria-label="Multi-base explorer"
      className="flex min-h-0 flex-col gap-2 border-b p-3 group-data-[collapsible=icon]:hidden"
    >
      <div className="flex items-center justify-between gap-2">
        <h2 className="text-sm font-medium">Connections</h2>
        <Button size="xs" variant="ghost" onClick={p.onRefresh}>
          Refresh list
        </Button>
      </div>
      {p.loading && (
        <p role="status" className="flex items-center gap-2 text-xs">
          <Spinner />
          Loading connections…
        </p>
      )}
      {p.error && (
        <BackendErrorAlert title="Explorer unavailable" error={p.error} />
      )}
      {!p.loading &&
        !p.error &&
        p.groups.length === 0 &&
        p.ungrouped.length === 0 && (
          <Empty>
            <EmptyHeader>
              <EmptyTitle>No saved connections</EmptyTitle>
              <EmptyDescription>
                Open a connection from the start screen to begin.
              </EmptyDescription>
            </EmptyHeader>
          </Empty>
        )}
      <div className="max-h-[38vh] overflow-y-auto">
        {p.groups.map((group, i) => (
          <section
            key={group.id}
            aria-label={group.name}
            className="flex flex-col gap-2 py-1"
          >
            <div className="flex items-center gap-1">
              <Button
                variant="ghost"
                size="xs"
                aria-expanded={group.expanded}
                aria-controls={`${prefix}-${i}`}
                aria-label={`Expand ${group.name}`}
                onClick={() => p.onToggle(group.id)}
              >
                {group.expanded ? "−" : "+"}
              </Button>
              <Button
                variant={
                  p.selected === `profile:${group.id}` ? "secondary" : "ghost"
                }
                className="min-w-0 flex-1 justify-start"
                onClick={() => p.onSelect(`profile:${group.id}`)}
              >
                <bdi className="truncate">{group.name}</bdi>
              </Button>
            </div>
            {group.expanded && (
              <div
                id={`${prefix}-${i}`}
                className="flex flex-col gap-2 border-l pl-2"
              >
                <ul className="flex flex-col gap-1">
                  {group.members.map((member) => row(member, true))}
                </ul>
                {group.members.length === 0 && (
                  <p className="text-xs text-muted-foreground">
                    No saved connections in this group.
                  </p>
                )}
                <div className="flex flex-wrap gap-1">
                  {group.discovery.state === "loading" ||
                  group.discovery.state === "cancelling" ? (
                    <Button
                      size="xs"
                      variant="outline"
                      disabled={group.discovery.state === "cancelling"}
                      onClick={() => p.onCancel(`discovery:${group.id}`)}
                    >
                      Cancel discovery
                    </Button>
                  ) : (
                    <Button
                      size="xs"
                      variant="outline"
                      disabled={!group.seed}
                      onClick={() => p.onDiscover(group.id)}
                    >
                      Read databases
                    </Button>
                  )}
                  <Button
                    size="xs"
                    variant="ghost"
                    disabled={!p.canGroup}
                    onClick={() => p.onAttach(group.id)}
                  >
                    Add active connection
                  </Button>
                  <Button
                    size="xs"
                    variant="ghost"
                    onClick={() => p.onRenameGroup(group.id)}
                  >
                    Rename group
                  </Button>
                  <Button
                    size="xs"
                    variant="ghost"
                    onClick={() => p.onDisconnectGroup(group.id)}
                  >
                    Disconnect group…
                  </Button>
                  <Button
                    size="xs"
                    variant="ghost"
                    onClick={() => p.onRemoveGroup(group.id)}
                  >
                    Remove group
                  </Button>
                </div>
                {group.seed && (
                  <p className="text-xs text-muted-foreground">
                    Discovery source: <bdi>{group.seed}</bdi>
                  </p>
                )}
                {!group.seed && (
                  <p className="text-xs text-muted-foreground">
                    Open a grouped connection first to read its server.
                  </p>
                )}
                {group.discovery.state === "initial" && (
                  <p className="text-xs text-muted-foreground">
                    Database list not loaded.
                  </p>
                )}
                {group.discovery.state === "loading" && (
                  <p role="status" className="flex gap-2 text-xs">
                    <Spinner />
                    Reading databases…
                  </p>
                )}
                {group.discovery.error && (
                  <BackendErrorAlert
                    title="Database discovery failed"
                    error={group.discovery.error}
                  />
                )}
                {group.discovery.state === "ready" &&
                  group.discovery.databases.length === 0 && (
                    <Empty>
                      <EmptyHeader>
                        <EmptyTitle>No databases returned</EmptyTitle>
                        <EmptyDescription>
                          The server returned an empty discovery list.
                        </EmptyDescription>
                      </EmptyHeader>
                    </Empty>
                  )}
                <ul className="flex flex-col gap-1">
                  {group.discovery.databases.map((db) => {
                    const key = JSON.stringify([group.id, db.name])
                    return (
                      <li key={db.name} className="flex flex-col gap-1">
                        <Button
                          variant={p.selected === key ? "secondary" : "ghost"}
                          aria-pressed={p.selected === key}
                          className="justify-start"
                          onClick={() => p.onSelect(key)}
                          onDoubleClick={() =>
                            p.onDatabase(group.id, db.name, false)
                          }
                          onKeyDown={(e) => {
                            if (e.key === "Enter") {
                              e.preventDefault()
                              p.onDatabase(group.id, db.name, false)
                            }
                          }}
                        >
                          <bdi className="truncate">{db.name}</bdi>
                        </Button>
                        {db.state && (
                          <p role="status" className="text-xs">
                            {db.state === "opening"
                              ? "Opening…"
                              : "Cancelling…"}
                          </p>
                        )}
                        {db.state && (
                          <Button
                            size="xs"
                            variant="outline"
                            disabled={db.state === "cancelling"}
                            onClick={() => p.onCancel(key)}
                          >
                            Cancel opening
                          </Button>
                        )}
                        {db.error && (
                          <BackendErrorAlert
                            title="Database not opened"
                            error={db.error}
                          />
                        )}
                        {!db.can_connect && (
                          <p className="text-xs text-muted-foreground">
                            Discovery account lacks CONNECT. Another account may
                            work.
                          </p>
                        )}
                        {p.selected === key && (
                          <div className="flex flex-wrap gap-1">
                            <Button
                              size="xs"
                              variant="outline"
                              onClick={() =>
                                p.onDatabase(group.id, db.name, false)
                              }
                            >
                              Open workspace
                            </Button>
                            <Button
                              size="xs"
                              variant="ghost"
                              onClick={() =>
                                p.onDatabase(group.id, db.name, true)
                              }
                            >
                              New connection
                            </Button>
                          </div>
                        )}
                      </li>
                    )
                  })}
                </ul>
              </div>
            )}
          </section>
        ))}
        {p.ungrouped.length > 0 && (
          <>
            <Separator />
            <h3 className="py-2 text-xs text-muted-foreground">Ungrouped</h3>
            <ul className="flex flex-col gap-1">
              {p.ungrouped.map((member) => row(member))}
            </ul>
          </>
        )}
      </div>
      {p.canGroup && (
        <Button size="xs" variant="outline" onClick={p.onCreateGroup}>
          Group this PostgreSQL server
        </Button>
      )}
      <p className="text-xs text-muted-foreground">
        Select a row; Enter or Open workspace activates it. Objects below belong
        to the active workspace.
      </p>
    </section>
  )
}
