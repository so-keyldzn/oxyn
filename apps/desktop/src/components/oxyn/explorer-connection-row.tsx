import type { CSSProperties, Ref } from "react"
import { HugeiconsIcon } from "@hugeicons/react"
import {
  LockIcon,
  MoreHorizontalIcon,
  StopIcon,
} from "@hugeicons/core-free-icons"
import { Button } from "@/components/ui/button"
import { Badge } from "@/components/ui/badge"
import { Spinner } from "@/components/ui/spinner"
import {
  DropdownMenu,
  DropdownMenuTrigger,
  DropdownMenuContent,
  DropdownMenuGroup,
  DropdownMenuLabel,
  DropdownMenuItem,
  DropdownMenuSeparator,
} from "@/components/ui/dropdown-menu"
import { BackendErrorAlert } from "./backend-error-alert"
import { DriverLogo } from "./driver-logo"
import { EnvironmentBadge } from "./environment-badge"
import type {
  ExplorerConnection,
  MultiBaseExplorerProps,
} from "./multi-base-explorer"

type Actions = Pick<
  MultiBaseExplorerProps,
  "onSelect" | "onActivate" | "onCancel" | "onDisconnect" | "onUnlink"
>
/** A compact destination identity; selecting never opens or switches a workspace. */
export function ExplorerConnectionRow({
  connection,
  selected,
  grouped = false,
  actions,
  disambiguate = false,
  style,
  measureRef,
  virtualIndex,
  total,
  onMenuOpenChange,
}: {
  connection: ExplorerConnection
  selected: boolean
  grouped?: boolean
  actions: Actions
  disambiguate?: boolean
  style?: CSSProperties
  measureRef?: Ref<HTMLLIElement>
  virtualIndex?: number
  total?: number
  onMenuOpenChange?: (open: boolean) => void
}) {
  const key = `connection:${connection.id}`
  const disambiguator = `${connection.location ? `${connection.location} · ` : ""}${connection.id.slice(-6)}`
  const busy =
    connection.state === "opening" || connection.state === "cancelling"
  const activateLabel =
    connection.state === "open"
      ? "Switch workspace"
      : connection.state === "active"
        ? "Return to workspace"
        : "Open workspace"
  return (
    <li
      className="flex min-w-0 flex-col gap-1"
      data-connection-row={connection.id}
      ref={measureRef}
      style={style}
      data-index={virtualIndex}
      aria-posinset={virtualIndex === undefined ? undefined : virtualIndex + 1}
      aria-setsize={total}
    >
      <div className="flex min-w-0 items-center gap-1">
        <Button
          variant={selected ? "secondary" : "ghost"}
          size="sm"
          className="h-8 min-w-0 flex-1 justify-start"
          data-connection-select
          aria-pressed={selected}
          aria-description={`${connection.environment}${disambiguate ? ` · ${disambiguator}` : ""}`}
          aria-current={connection.state === "active" ? "page" : undefined}
          aria-label={`${connection.name} — ${connection.state}${connection.readOnly ? ", read only" : ""}`}
          onClick={() => actions.onSelect(key)}
          onDoubleClick={() => actions.onActivate(connection.id)}
          onKeyDown={(event) => {
            if (event.key === "Enter") {
              event.preventDefault()
              actions.onActivate(connection.id)
            }
          }}
        >
          <DriverLogo
            driver={connection.driver ?? ""}
            className="size-4 shrink-0"
          />
          <bdi className="min-w-0 flex-1 truncate text-left">
            {connection.name}
            {disambiguate && (
              <span className="text-muted-foreground"> · {disambiguator}</span>
            )}
          </bdi>
          {connection.environment !== "local" &&
            connection.environment !== "development" && (
              <EnvironmentBadge environment={connection.environment} />
            )}
          {connection.readOnly && (
            <HugeiconsIcon
              icon={LockIcon}
              data-icon="inline-end"
              strokeWidth={2}
            />
          )}
          {busy ? (
            <Spinner data-icon="inline-end" />
          ) : connection.state === "active" ? (
            <Badge variant="secondary">Active</Badge>
          ) : connection.state === "open" ? (
            <span className="text-xs text-muted-foreground">Open</span>
          ) : null}
        </Button>
        {busy ? (
          <Button
            variant="ghost"
            size="icon-xs"
            aria-label={`Cancel opening ${connection.name}${disambiguate ? ` · ${disambiguator}` : ""}`}
            disabled={connection.state === "cancelling"}
            onClick={() => actions.onCancel(key)}
          >
            <HugeiconsIcon icon={StopIcon} strokeWidth={2} />
          </Button>
        ) : (
          <DropdownMenu modal={false} onOpenChange={onMenuOpenChange}>
            <DropdownMenuTrigger
              render={
                <Button
                  variant="ghost"
                  size="icon-xs"
                  aria-label={`Actions for ${connection.name}${disambiguate ? ` · ${disambiguator}` : ""}`}
                />
              }
            >
              <HugeiconsIcon icon={MoreHorizontalIcon} strokeWidth={2} />
            </DropdownMenuTrigger>
            <DropdownMenuContent align="end" className="max-w-72">
              <DropdownMenuGroup>
                <DropdownMenuLabel>
                  <span className="flex flex-col gap-1">
                    <bdi className="wrap-anywhere">{connection.name}</bdi>
                    <span className="wrap-anywhere">
                      {connection.location ?? "Destination not specified"}
                    </span>
                    <span>
                      {connection.environment}
                      {connection.readOnly ? " · Read only" : ""}
                    </span>
                    {connection.transaction && (
                      <span>{connection.transaction}</span>
                    )}
                  </span>
                </DropdownMenuLabel>
                <DropdownMenuItem
                  onClick={() => actions.onActivate(connection.id)}
                >
                  {activateLabel}
                </DropdownMenuItem>
              </DropdownMenuGroup>
              {(connection.state === "active" ||
                connection.state === "open" ||
                grouped) && (
                <>
                  <DropdownMenuSeparator />
                  <DropdownMenuGroup>
                    {(connection.state === "active" ||
                      connection.state === "open") && (
                      <DropdownMenuItem
                        onClick={() => actions.onDisconnect(connection.id)}
                      >
                        Disconnect…
                      </DropdownMenuItem>
                    )}
                    {grouped && (
                      <DropdownMenuItem
                        onClick={() => actions.onUnlink(connection.id)}
                      >
                        Ungroup
                      </DropdownMenuItem>
                    )}
                  </DropdownMenuGroup>
                </>
              )}
            </DropdownMenuContent>
          </DropdownMenu>
        )}
      </div>
      {connection.transaction && (
        <p className="text-xs text-muted-foreground">
          {connection.transaction}
        </p>
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
