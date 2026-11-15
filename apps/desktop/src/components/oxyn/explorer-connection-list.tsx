import * as React from "react"
import { defaultRangeExtractor, useVirtualizer } from "@tanstack/react-virtual"
import { ExplorerConnectionRow } from "./explorer-connection-row"
import type {
  ExplorerConnection,
  MultiBaseExplorerProps,
} from "./multi-base-explorer"

type Actions = Pick<
  MultiBaseExplorerProps,
  "onSelect" | "onActivate" | "onCancel" | "onDisconnect" | "onUnlink"
>
/** Only visible saved destinations mount menus; focused/open-menu identities remain mounted. */
export function ExplorerConnectionList({
  connections,
  selected,
  grouped = false,
  actions,
  nameCounts,
  label,
  filterKey,
  outerViewport,
}: {
  connections: Array<ExplorerConnection>
  selected: string | null
  grouped?: boolean
  actions: Actions
  nameCounts: ReadonlyMap<string, number>
  label: string
  filterKey: string
  outerViewport?: React.RefObject<HTMLDivElement | null>
}) {
  const scroll = React.useRef<HTMLDivElement>(null)
  const [focused, setFocused] = React.useState<string | null>(null)
  const [menu, setMenu] = React.useState<string | null>(null)
  const [visible, setVisible] = React.useState(
    !outerViewport || typeof IntersectionObserver === "undefined"
  )
  React.useEffect(() => {
    if (
      !outerViewport ||
      typeof IntersectionObserver === "undefined" ||
      !scroll.current
    )
      return
    const observer = new IntersectionObserver(
      (entries) => setVisible(entries.some((entry) => entry.isIntersecting)),
      { root: outerViewport.current, rootMargin: "64px 0px" }
    )
    observer.observe(scroll.current)
    return () => observer.disconnect()
  }, [outerViewport])
  const enabled = visible || focused !== null || menu !== null
  const placeholderHeight = React.useRef(connections.length * 36)
  const menuOpen = React.useRef(false)
  const pendingFocus = React.useRef<string | null>(null)
  const pinned = [focused, menu].flatMap((id) => {
    const index =
      id === null
        ? -1
        : connections.findIndex((connection) => connection.id === id)
    return index < 0 ? [] : [index]
  })
  const virtual = useVirtualizer({
    count: connections.length,
    enabled,
    getScrollElement: () => scroll.current,
    getItemKey: (index) => connections[index]?.id ?? index,
    estimateSize: () => 36,
    overscan: 6,
    rangeExtractor: (range) =>
      [...new Set([...defaultRangeExtractor(range), ...pinned])].sort(
        (a, b) => a - b
      ),
  })
  React.useEffect(() => {
    virtual.scrollToOffset(0)
  }, [filterKey])
  if (enabled) placeholderHeight.current = virtual.getTotalSize()
  const focusPending = () => {
    const id = pendingFocus.current
    if (!id) return
    const row = [
      ...(scroll.current?.querySelectorAll<HTMLElement>(
        "[data-connection-row]"
      ) ?? []),
    ].find((node) => node.dataset.connectionRow === id)
    const button = row?.querySelector<HTMLButtonElement>(
      "[data-connection-select]"
    )
    if (button) {
      pendingFocus.current = null
      button.focus()
    }
  }
  React.useLayoutEffect(focusPending, [focused, connections])
  return (
    <div
      ref={scroll}
      role="region"
      aria-label={label}
      className="max-h-[18dvh] min-h-0 overflow-y-auto"
      style={{ height: Math.min(160, connections.length * 36) }}
      onFocusCapture={(event) => {
        const row = (event.target as HTMLElement).closest<HTMLElement>(
          "[data-connection-row]"
        )
        if (row) {
          setFocused(row.dataset.connectionRow ?? null)
          if (!menuOpen.current) setMenu(null)
        }
      }}
      onBlurCapture={(event) => {
        if (
          !menuOpen.current &&
          !event.currentTarget.contains(event.relatedTarget)
        ) {
          setFocused(null)
          setMenu(null)
        }
      }}
      onKeyDown={(event) => {
        const button = (event.target as HTMLElement).closest<HTMLButtonElement>(
          "[data-connection-select]"
        )
        if (!button) return
        const current = button.closest<HTMLElement>("[data-connection-row]")
          ?.dataset.connectionRow
        const index = connections.findIndex(
          (connection) => connection.id === current
        )
        const target =
          event.key === "ArrowDown"
            ? Math.min(connections.length - 1, index + 1)
            : event.key === "ArrowUp"
              ? Math.max(0, index - 1)
              : event.key === "Home"
                ? 0
                : event.key === "End"
                  ? connections.length - 1
                  : null
        if (target === null || index < 0) return
        const next = connections[target]
        if (!next) return
        event.preventDefault()
        pendingFocus.current = next.id
        setFocused(next.id)
        virtual.scrollToIndex(target, { align: "auto" })
      }}
    >
      <ul
        className="relative w-full"
        style={{
          height: enabled ? virtual.getTotalSize() : placeholderHeight.current,
        }}
      >
        {virtual.getVirtualItems().map((item) => {
          const connection = connections[item.index]
          return connection ? (
            <ExplorerConnectionRow
              key={connection.id}
              connection={connection}
              selected={selected === `connection:${connection.id}`}
              grouped={grouped}
              actions={actions}
              disambiguate={(nameCounts.get(connection.name) ?? 0) > 1}
              virtualIndex={item.index}
              total={connections.length}
              measureRef={(node) => {
                virtual.measureElement(node)
                if (node && pendingFocus.current === connection.id)
                  focusPending()
              }}
              style={{
                position: "absolute",
                top: 0,
                left: 0,
                width: "100%",
                transform: `translateY(${item.start}px)`,
              }}
              onMenuOpenChange={(open) => {
                menuOpen.current = open
                setMenu(connection.id)
              }}
            />
          ) : null
        })}
      </ul>
    </div>
  )
}
