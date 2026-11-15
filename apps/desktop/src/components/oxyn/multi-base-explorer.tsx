import * as React from "react"
import { HugeiconsIcon } from "@hugeicons/react"
import {
  ArrowDown01Icon,
  ArrowRight01Icon,
  MoreHorizontalIcon,
  Search01Icon,
  ArrowReloadHorizontalIcon,
} from "@hugeicons/core-free-icons"
import {
  DropdownMenu,
  DropdownMenuTrigger,
  DropdownMenuContent,
  DropdownMenuGroup,
  DropdownMenuItem,
  DropdownMenuSeparator,
} from "@/components/ui/dropdown-menu"
import { Button } from "@/components/ui/button"
import { TextInput } from "./text-field"
import { Field, FieldLabel } from "@/components/ui/field"
import { ExplorerConnectionList } from "./explorer-connection-list"
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
import type { Environment } from "@/lib/ipc/types"

export interface ExplorerConnection {
  id: string
  name: string
  location: string | null
  driver?: string | null
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
  onSkipToCatalog?: () => void
}

/** Navigation selection never changes the active workspace or its protections. */
export function MultiBaseExplorer(p: MultiBaseExplorerProps) {
  const prefix = React.useId()
  const savedViewport = React.useRef<HTMLDivElement>(null)
  const [filterOpen, setFilterOpen] = React.useState(false)
  const [filter, setFilter] = React.useState("")
  const filterId = React.useId()
  const searchTrigger = React.useRef<HTMLButtonElement>(null)
  const query = filter.trim().toLocaleLowerCase()
  const matches = (connection: ExplorerConnection) =>
    !query ||
    `${connection.name} ${connection.location ?? ""}`
      .toLocaleLowerCase()
      .includes(query)
  const groups = p.groups
    .map((group) => ({
      ...group,
      members: group.name.toLocaleLowerCase().includes(query)
        ? group.members
        : group.members.filter(matches),
    }))
    .filter(
      (group) =>
        !query ||
        group.members.length > 0 ||
        group.name.toLocaleLowerCase().includes(query)
    )
  const ungrouped = p.ungrouped.filter(matches)
  const count =
    p.ungrouped.length +
    p.groups.reduce((total, group) => total + group.members.length, 0)
  const nameCounts = new Map<string, number>()
  for (const connection of [
    ...p.ungrouped,
    ...p.groups.flatMap((group) => group.members),
  ]) {
    nameCounts.set(connection.name, (nameCounts.get(connection.name) ?? 0) + 1)
  }
  return (
    <section
      aria-label="Multi-base explorer"
      className="flex max-h-[min(28dvh,16rem)] min-h-0 shrink-0 flex-col gap-1 border-b px-2 pt-8 pb-2 group-data-[collapsible=icon]:hidden"
    >
      <div className="flex items-center justify-between gap-2">
        <h2 className="text-sm font-medium">
          Connections{!p.loading && count > 0 ? ` · ${count}` : ""}
        </h2>
        <div className="flex gap-1">
          <Button
            size="icon-xs"
            variant="ghost"
            ref={searchTrigger}
            aria-label="Find a saved connection"
            aria-expanded={filterOpen}
            aria-controls={filterId}
            onClick={() => {
              setFilterOpen(!filterOpen)
              if (filterOpen) setFilter("")
            }}
          >
            <HugeiconsIcon icon={Search01Icon} strokeWidth={2} />
          </Button>
          <Button
            size="icon-xs"
            variant="ghost"
            aria-label="Refresh list"
            onClick={p.onRefresh}
          >
            <HugeiconsIcon icon={ArrowReloadHorizontalIcon} strokeWidth={2} />
          </Button>
          {p.canGroup && (
            <DropdownMenu modal={false}>
              <DropdownMenuTrigger
                render={
                  <Button
                    size="icon-xs"
                    variant="ghost"
                    aria-label="Connection actions"
                  />
                }
              >
                <HugeiconsIcon icon={MoreHorizontalIcon} strokeWidth={2} />
              </DropdownMenuTrigger>
              <DropdownMenuContent align="end">
                <DropdownMenuGroup>
                  <DropdownMenuItem onClick={p.onCreateGroup}>
                    Group this PostgreSQL server
                  </DropdownMenuItem>
                </DropdownMenuGroup>
              </DropdownMenuContent>
            </DropdownMenu>
          )}
        </div>
      </div>
      {p.onSkipToCatalog && (
        <Button
          variant="ghost"
          size="sm"
          className="sr-only focus:not-sr-only"
          onClick={p.onSkipToCatalog}
        >
          Skip to database objects
        </Button>
      )}
      {filterOpen && (
        <Field>
          <FieldLabel htmlFor={filterId} className="sr-only">
            Filter saved connections
          </FieldLabel>
          <TextInput
            id={filterId}
            autoFocus
            value={filter}
            placeholder="Find a connection…"
            onChange={(event) => setFilter(event.target.value)}
            onKeyDown={(event) => {
              if (event.key === "Escape") {
                setFilter("")
                setFilterOpen(false)
                searchTrigger.current?.focus()
              }
            }}
          />
        </Field>
      )}
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
      <div
        className="min-h-0 overflow-y-auto"
        ref={savedViewport}
        role="region"
        aria-label="Saved connections"
      >
        {groups.map((group, i) => (
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
                <HugeiconsIcon
                  icon={group.expanded ? ArrowDown01Icon : ArrowRight01Icon}
                  strokeWidth={2}
                />
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
              <DropdownMenu modal={false}>
                <DropdownMenuTrigger
                  render={
                    <Button
                      variant="ghost"
                      size="icon-xs"
                      aria-label={`Manage ${group.name}`}
                    />
                  }
                >
                  <HugeiconsIcon icon={MoreHorizontalIcon} strokeWidth={2} />
                </DropdownMenuTrigger>
                <DropdownMenuContent align="end">
                  <DropdownMenuGroup>
                    <DropdownMenuItem
                      disabled={!p.canGroup}
                      onClick={() => p.onAttach(group.id)}
                    >
                      Add active connection
                    </DropdownMenuItem>
                    <DropdownMenuItem onClick={() => p.onRenameGroup(group.id)}>
                      Rename group
                    </DropdownMenuItem>
                  </DropdownMenuGroup>
                  <DropdownMenuSeparator />
                  <DropdownMenuGroup>
                    <DropdownMenuItem
                      onClick={() => p.onDisconnectGroup(group.id)}
                    >
                      Disconnect group…
                    </DropdownMenuItem>
                    <DropdownMenuItem onClick={() => p.onRemoveGroup(group.id)}>
                      Remove group
                    </DropdownMenuItem>
                  </DropdownMenuGroup>
                </DropdownMenuContent>
              </DropdownMenu>
            </div>
            {group.expanded && (
              <div
                id={`${prefix}-${i}`}
                className="flex flex-col gap-2 border-l pl-2"
              >
                <ExplorerConnectionList
                  connections={group.members}
                  selected={p.selected}
                  grouped
                  actions={p}
                  nameCounts={nameCounts}
                  label={`${group.name} saved connections`}
                  filterKey={query}
                  outerViewport={savedViewport}
                />
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
        {ungrouped.length > 0 && (
          <>
            {p.groups.length > 0 && (
              <>
                <Separator />
                <h3 className="py-1 text-xs text-muted-foreground">
                  Other connections
                </h3>
              </>
            )}
            <ExplorerConnectionList
              connections={ungrouped}
              selected={p.selected}
              actions={p}
              nameCounts={nameCounts}
              label="Ungrouped saved connections"
              filterKey={query}
              outerViewport={savedViewport}
            />
          </>
        )}
      </div>

      {query && groups.length === 0 && ungrouped.length === 0 && (
        <p role="status" className="text-xs text-muted-foreground">
          No matching saved connections.
        </p>
      )}
    </section>
  )
}
