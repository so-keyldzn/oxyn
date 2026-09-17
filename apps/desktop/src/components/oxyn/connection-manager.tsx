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
import { Skeleton } from "@/components/ui/skeleton"
import type { ConnectionSummary } from "@/lib/ipc/settings"

/**
 * The saved connections, to edit or delete — never to open.
 *
 * The connection in use cannot be deleted from here: its workspace would be
 * left running against a configuration that no longer exists. Leaving it
 * first is one click, and says what happens.
 */
export function ConnectionManager({
  connections,
  error,
  openConnectionId = null,
  onEdit,
  onDelete,
  onRetry,
}: {
  connections: Array<ConnectionSummary> | undefined
  error?: BackendFailure | null
  openConnectionId?: string | null
  onEdit: (connection: ConnectionSummary) => void
  onDelete: (connection: ConnectionSummary) => void
  onRetry?: () => void
}) {
  const baseId = React.useId()
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

  return (
    <ItemGroup className="gap-2">
      {connections.map((connection, index) => {
        const inUse = connection.id === openConnectionId
        const inUseId = `${baseId}-in-use-${index}`
        const described = connection.location
          ? `${connection.driverName} · ${connection.location}`
          : connection.driverName
        return (
          <Item key={connection.id} variant="outline" role="listitem">
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
                  In use: leave this connection to delete it.
                </ItemDescription>
              ) : null}
            </ItemContent>
            <ItemActions className="ms-auto shrink-0">
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
  )
}
