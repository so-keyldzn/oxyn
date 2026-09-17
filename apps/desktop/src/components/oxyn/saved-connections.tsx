import * as React from "react"
import { HugeiconsIcon } from "@hugeicons/react"
import {
  ArrowRight01Icon,
  DatabaseIcon,
  LockIcon,
} from "@hugeicons/core-free-icons"

import { BackendErrorAlert } from "@/components/oxyn/backend-error-alert"
import type { BackendFailure } from "@/components/oxyn/backend-error-alert"
import { EnvironmentBadge } from "@/components/oxyn/environment-badge"
import { Button } from "@/components/ui/button"
import {
  Empty,
  EmptyDescription,
  EmptyHeader,
  EmptyMedia,
  EmptyTitle,
} from "@/components/ui/empty"
import {
  Item,
  ItemActions,
  ItemContent,
  ItemDescription,
  ItemGroup,
  ItemMedia,
  ItemTitle,
} from "@/components/ui/item"
import { Kbd } from "@/components/ui/kbd"
import { Skeleton } from "@/components/ui/skeleton"
import { Spinner } from "@/components/ui/spinner"
import type { ConnectionSummary } from "@/lib/ipc/settings"

/**
 * The connections of the workspace. Opening one is always the user's click or
 * Enter; ↑↓ move between them.
 *
 * Two connections may share a name: the driver and the location line — host,
 * database or file name, never a secret — tell them apart. While one opens,
 * the others are inert and the opening one offers Cancel (Esc).
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

  return (
    <ItemGroup ref={listRef} className="gap-2" onKeyDown={moveFocus}>
      {connections.map((connection) => {
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
              <ItemMedia variant="icon">
                <HugeiconsIcon icon={DatabaseIcon} strokeWidth={2} />
              </ItemMedia>
              <ItemContent className="min-w-0">
                <ItemTitle className="w-full min-w-0">
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
                </ItemTitle>
                {/* The title sits on the line that is cut, so hovering any
                    part of it — driver included — reads the whole line. */}
                <ItemDescription
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
                </ItemDescription>
              </ItemContent>
              <ItemActions>
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
              </ItemActions>
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
}
