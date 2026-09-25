import * as React from "react"
import { HugeiconsIcon } from "@hugeicons/react"
import {
  DatabaseIcon,
  Delete02Icon,
  Edit02Icon,
  LockIcon,
} from "@hugeicons/core-free-icons"

import { BackendErrorAlert } from "@/components/oxyn/backend-error-alert"
import type { BackendFailure } from "@/components/oxyn/backend-error-alert"
import { EnvironmentBadge } from "@/components/oxyn/environment-badge"
import { PrivacyTierBadge } from "@/components/oxyn/privacy-tier"
import { ActionMenuContent } from "@/components/oxyn/action-menu-items"
import { Button } from "@/components/ui/button"
import { ContextMenu, ContextMenuTrigger } from "@/components/ui/context-menu"
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
import { Skeleton } from "@/components/ui/skeleton"
import type { ConnectionMenuActions } from "@/lib/actions/targets"
import type { ConnectionSummary } from "@/lib/ipc/settings"

/**
 * The saved connections, to edit or delete — never to open.
 *
 * A connection open in the window — its workspace shown or hidden — cannot be
 * deleted from here: that workspace would be left running against a
 * configuration that no longer exists. Disconnecting it first is one click,
 * and says what happens.
 *
 * A right click — or ⇧F10 on a focused row — opens the row's context menu:
 * `Edit…` and `Delete…` are the two buttons, the rest is what `menuActions`
 * gives. Nothing opens or closes a connection from here, so `Connect`,
 * `Disconnect`, `New console` and `Refresh catalog` are not offered.
 */
export function ConnectionManager({
  connections,
  error,
  openConnectionIds = [],
  onEdit,
  onDelete,
  onRetry,
  menuActions,
}: {
  connections: Array<ConnectionSummary> | undefined
  error?: BackendFailure | null
  /** Every connection with a workspace in the window, shown or hidden. */
  openConnectionIds?: ReadonlyArray<string>
  onEdit: (connection: ConnectionSummary) => void
  onDelete: (connection: ConnectionSummary) => void
  onRetry?: () => void
  menuActions?: (connection: ConnectionSummary) => ConnectionMenuActions
}) {
  const baseId = React.useId()
  const [menu, setMenu] = React.useState<{
    connection: ConnectionSummary
    anchor: Element
  } | null>(null)
  if (error) {
    return (
      <BackendErrorAlert
        title="Cannot list connections"
        error={error}
        onRetry={onRetry}
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
            Connections created from the start screen appear here.
          </EmptyDescription>
        </EmptyHeader>
      </Empty>
    )
  }

  // By position, as drawn: the connection's id never reaches the DOM (I-03).
  const pickTarget = (event: React.MouseEvent) => {
    const anchor = event.target instanceof Element ? event.target : null
    const row = anchor?.closest("[data-connection-row]")
    const index = row ? Number(row.getAttribute("data-connection-row")) : -1
    const connection = connections[index]
    setMenu(anchor && connection ? { connection, anchor } : null)
  }
  const menuInUse =
    menu !== null && openConnectionIds.includes(menu.connection.id)

  return (
    <ContextMenu>
      <ContextMenuTrigger
        render={<div className="contents" onContextMenu={pickTarget} />}
      >
        <ItemGroup className="gap-2">
          {connections.map((connection, index) => {
            const inUse = openConnectionIds.includes(connection.id)
            const inUseId = `${baseId}-in-use-${index}`
            const described = connection.location
              ? `${connection.driverName} · ${connection.location}`
              : connection.driverName
            return (
              <Item
                key={connection.id}
                variant="outline"
                role="listitem"
                data-connection-row={index}
              >
                <ItemMedia variant="icon">
                  <HugeiconsIcon icon={DatabaseIcon} strokeWidth={2} />
                </ItemMedia>
                {/* A real basis, not `flex-1`'s zero: the item wraps its actions
                onto a second line once the name has less than 10 rem, rather
                than pushing them past a 420 px window. */}
                <ItemContent className="min-w-0 basis-40">
                  <ItemTitle className="w-full min-w-0">
                    <bdi className="truncate" title={connection.name}>
                      {connection.name}
                    </bdi>
                    {connection.readOnly ? (
                      <HugeiconsIcon
                        icon={LockIcon}
                        strokeWidth={2}
                        className="size-3.5 text-muted-foreground"
                        aria-label="Read only"
                      />
                    ) : null}
                  </ItemTitle>
                  <ItemDescription className="truncate" title={described}>
                    {connection.driverName}
                    {connection.location ? (
                      <>
                        {" · "}
                        <bdi>{connection.location}</bdi>
                      </>
                    ) : null}
                  </ItemDescription>
                  {inUse ? (
                    // Its own line, never cut: it is the only reason given for a
                    // disabled Delete, and the button points at it.
                    <ItemDescription id={inUseId}>
                      Open in this window: disconnect it to delete it.
                    </ItemDescription>
                  ) : null}
                </ItemContent>
                {/* Once on a line of their own, the badges and buttons wrap
                rather than drawing past a 320 px dialog. */}
                <ItemActions className="ms-auto min-w-0 flex-wrap justify-end">
                  <EnvironmentBadge environment={connection.environment} />
                  <PrivacyTierBadge tier={connection.privacyTier} />
                  <Button
                    variant="ghost"
                    size="icon-sm"
                    aria-label={`Edit ${connection.name}`}
                    onClick={() => onEdit(connection)}
                  >
                    <HugeiconsIcon icon={Edit02Icon} strokeWidth={2} />
                  </Button>
                  <Button
                    variant="ghost"
                    size="icon-sm"
                    aria-label={`Delete ${connection.name}`}
                    aria-describedby={inUse ? inUseId : undefined}
                    disabled={inUse}
                    onClick={() => onDelete(connection)}
                  >
                    <HugeiconsIcon icon={Delete02Icon} strokeWidth={2} />
                  </Button>
                </ItemActions>
              </Item>
            )
          })}
        </ItemGroup>
      </ContextMenuTrigger>
      {menu ? (
        <ActionMenuContent
          surface="connection"
          anchor={menu.anchor}
          title={menu.connection.name}
          sources={{
            connection: {
              state: { open: menuInUse, busy: false },
              actions: {
                ...menuActions?.(menu.connection),
                edit: () => onEdit(menu.connection),
                // As the button: the connection in use is left first.
                delete: menuInUse ? undefined : () => onDelete(menu.connection),
              },
            },
          }}
        />
      ) : null}
    </ContextMenu>
  )
}
