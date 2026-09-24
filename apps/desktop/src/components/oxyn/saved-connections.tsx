import * as React from "react"
import { HugeiconsIcon } from "@hugeicons/react"
import {
  ArrowRight01Icon,
  SearchIcon,
  DatabaseIcon,
  LockIcon,
} from "@hugeicons/core-free-icons"

import { BackendErrorAlert } from "@/components/oxyn/backend-error-alert"
import type { BackendFailure } from "@/components/oxyn/backend-error-alert"
import {
  ButtonItemActions,
  ButtonItemContent,
  ButtonItemDescription,
  ButtonItemMedia,
  ButtonItemTitle,
} from "@/components/oxyn/button-item"
import { DriverLogo } from "@/components/oxyn/driver-logo"
import { EnvironmentBadge } from "@/components/oxyn/environment-badge"
import { Button } from "@/components/ui/button"
import {
  Empty,
  EmptyContent,
  EmptyDescription,
  EmptyHeader,
  EmptyMedia,
  EmptyTitle,
} from "@/components/ui/empty"
import { Item, ItemGroup } from "@/components/ui/item"
import {
  InputGroup,
  InputGroupAddon,
  InputGroupInput,
} from "@/components/ui/input-group"
import { Kbd } from "@/components/ui/kbd"
import { Skeleton } from "@/components/ui/skeleton"
import { Spinner } from "@/components/ui/spinner"
import type { ConnectionSummary } from "@/lib/ipc/settings"

/**
 * Beyond this many connections the list is filtered and folded: a workspace
 * of forty connections must not push the database types off the screen.
 */
const SHOWN_FOLDED = 5

function matches(connection: ConnectionSummary, query: string) {
  const needle = query.trim().toLocaleLowerCase()
  if (!needle) return true
  return [connection.name, connection.driverName, connection.location ?? ""]
    .join(" ")
    .toLocaleLowerCase()
    .includes(needle)
}

/**
 * The connections of the workspace. Opening one is always the user's click or
 * Enter; ↑↓ move between them.
 *
 * Two connections may share a name: the driver and the location line — host,
 * database or file name, never a secret — tell them apart. While one opens,
 * the others are inert and the opening one offers Cancel (Esc).
 *
 * Past five, a filter (name, driver, location) sits above the list and only
 * five rows show until « Show all ». The one opening always stays in view.
 */
export function SavedConnections({
  connections,
  error,
  opening,
  cancelling = false,
  onOpen,
  onCancelOpening,
  onRetry,
}: {
  connections: Array<ConnectionSummary> | undefined
  error?: BackendFailure | null
  opening: string | null
  cancelling?: boolean
  onOpen: (connection: ConnectionSummary) => void
  onCancelOpening?: () => void
  onRetry?: () => void
}) {
  const listRef = React.useRef<HTMLDivElement>(null)
  const [query, setQuery] = React.useState("")
  const [expanded, setExpanded] = React.useState(false)

  React.useEffect(() => {
    if (opening === null || !onCancelOpening) return
    const onKey = (event: KeyboardEvent) => {
      if (event.key === "Escape" && !event.defaultPrevented) {
        event.preventDefault()
        onCancelOpening()
      }
    }
    window.addEventListener("keydown", onKey)
    return () => window.removeEventListener("keydown", onKey)
  }, [opening, onCancelOpening])

  if (error) {
    return (
      <BackendErrorAlert
        title="Cannot list connections"
        error={error}
        onRetry={onRetry}
        nextStep="The workspace state could not be read. Restart Oxyn if it persists."
      />
    )
  }

  if (connections === undefined) {
    return (
      <div className="flex flex-col gap-2" aria-busy="true">
        <span className="sr-only">Loading saved connections</span>
        {Array.from({ length: 3 }, (_, index) => (
          <Skeleton key={index} className="h-14 w-full" />
        ))}
      </div>
    )
  }

  if (connections.length === 0) {
    return (
      <Empty className="border">
        <EmptyHeader>
          <EmptyMedia variant="icon">
            <HugeiconsIcon icon={DatabaseIcon} strokeWidth={2} />
          </EmptyMedia>
          <EmptyTitle>No saved connection</EmptyTitle>
          <EmptyDescription>
            Choose a database type to create your first connection.
          </EmptyDescription>
        </EmptyHeader>
      </Empty>
    )
  }

  const long = connections.length > SHOWN_FOLDED
  const filtered = long
    ? connections.filter((connection) => matches(connection, query))
    : connections
  const folded = !expanded && query.trim() === ""
  const visible = folded
    ? filtered.filter(
        (connection, index) => index < SHOWN_FOLDED || connection.id === opening
      )
    : filtered
  const hidden = filtered.length - visible.length

  const moveFocus = (event: React.KeyboardEvent) => {
    if (event.key !== "ArrowDown" && event.key !== "ArrowUp") return
    const buttons = Array.from(
      listRef.current?.querySelectorAll<HTMLButtonElement>(
        "[data-connection-button]"
      ) ?? []
    )
    const index = buttons.findIndex(
      (button) => button === document.activeElement
    )
    const next = event.key === "ArrowDown" ? index + 1 : index - 1
    const target = buttons[Math.max(0, Math.min(buttons.length - 1, next))]
    if (target) {
      event.preventDefault()
      target.focus()
    }
  }

  const list = (
    <ItemGroup ref={listRef} className="gap-2" onKeyDown={moveFocus}>
      {visible.map((connection) => {
        const isOpening = opening === connection.id
        return (
          <div
            role="listitem"
            key={connection.id}
            className="flex items-center gap-2"
          >
            <Item
              variant="outline"
              render={
                <button
                  type="button"
                  data-connection-button
                  disabled={opening !== null}
                  aria-busy={isOpening || undefined}
                  onClick={() => onOpen(connection)}
                />
              }
              className="min-w-0 flex-1 text-left"
            >
              <ButtonItemMedia>
                <DriverLogo driver={connection.driver} className="size-4" />
              </ButtonItemMedia>
              <ButtonItemContent className="min-w-0">
                <ButtonItemTitle className="w-full min-w-0">
                  <bdi className="truncate" title={connection.name}>
                    {connection.name}
                  </bdi>
                  {connection.readOnly ? (
                    <HugeiconsIcon
                      icon={LockIcon}
                      strokeWidth={2}
                      className="size-3.5 shrink-0 text-muted-foreground"
                      aria-label="Read only"
                    />
                  ) : null}
                </ButtonItemTitle>
                {/* The title sits on the line that is cut, so hovering any
                    part of it — driver included — reads the whole line. */}
                <ButtonItemDescription
                  className="truncate"
                  title={
                    connection.location
                      ? `${connection.driverName} · ${connection.location}`
                      : connection.driverName
                  }
                >
                  {connection.driverName}
                  {connection.location ? (
                    <>
                      {" · "}
                      <bdi>{connection.location}</bdi>
                    </>
                  ) : null}
                </ButtonItemDescription>
              </ButtonItemContent>
              <ButtonItemActions>
                <EnvironmentBadge environment={connection.environment} />
                {isOpening ? (
                  <Spinner />
                ) : (
                  <HugeiconsIcon
                    icon={ArrowRight01Icon}
                    strokeWidth={2}
                    className="size-4 text-muted-foreground"
                  />
                )}
              </ButtonItemActions>
            </Item>
            {isOpening && onCancelOpening ? (
              <Button
                variant="outline"
                size="sm"
                onClick={onCancelOpening}
                disabled={cancelling}
                aria-label={`Cancel opening ${connection.name}`}
                // `Kbd` draws the key; the name says what the button does, so
                // the shortcut has to be declared rather than read from it.
                aria-keyshortcuts="Escape"
              >
                {cancelling ? "Cancelling…" : "Cancel"}
                {cancelling ? null : <Kbd>Esc</Kbd>}
              </Button>
            ) : null}
          </div>
        )
      })}
    </ItemGroup>
  )

  if (!long) return list

  return (
    <div className="flex flex-col gap-3">
      <InputGroup>
        <InputGroupInput
          type="search"
          value={query}
          onChange={(event) => setQuery(event.target.value)}
          onKeyDown={(event) => {
            // Esc empties the filter first; only an empty one lets it through.
            if (event.key === "Escape" && query !== "") {
              event.preventDefault()
              setQuery("")
            }
          }}
          placeholder={`Filter ${connections.length} connections…`}
          aria-label="Filter saved connections"
        />
        <InputGroupAddon>
          <HugeiconsIcon icon={SearchIcon} strokeWidth={2} />
        </InputGroupAddon>
      </InputGroup>
      {filtered.length === 0 ? (
        <Empty className="gap-3 border p-4">
          <EmptyHeader>
            <EmptyTitle className="w-full min-w-0 truncate">
              No connection matches <bdi>“{query.trim()}”</bdi>
            </EmptyTitle>
          </EmptyHeader>
          <EmptyContent>
            <Button variant="outline" size="sm" onClick={() => setQuery("")}>
              Clear filter
            </Button>
          </EmptyContent>
        </Empty>
      ) : (
        list
      )}
      {hidden > 0 ? (
        <Button
          variant="ghost"
          size="sm"
          className="self-start"
          onClick={() => setExpanded(true)}
        >
          Show all {connections.length} connections
        </Button>
      ) : null}
    </div>
  )
}
