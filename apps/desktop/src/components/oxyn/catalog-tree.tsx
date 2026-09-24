import * as React from "react"
import { useDebouncedValue } from "@tanstack/react-pacer"
import { useVirtualizer } from "@tanstack/react-virtual"
import { HugeiconsIcon } from "@hugeicons/react"
import {
  ArrowRight01Icon,
  DatabaseIcon,
  EyeIcon,
  FolderLibraryIcon,
  FunctionIcon,
  Search01Icon,
  TableIcon,
  ViewOffIcon,
} from "@hugeicons/core-free-icons"

import { Badge } from "@/components/ui/badge"
import {
  ContextMenu,
  ContextMenuContent,
  ContextMenuGroup,
  ContextMenuItem,
  ContextMenuLabel,
  ContextMenuSeparator,
  ContextMenuTrigger,
} from "@/components/ui/context-menu"
import {
  Empty,
  EmptyDescription,
  EmptyHeader,
  EmptyTitle,
} from "@/components/ui/empty"
import {
  InputGroup,
  InputGroupAddon,
  InputGroupInput,
} from "@/components/ui/input-group"
import { Spinner } from "@/components/ui/spinner"
import { Toggle } from "@/components/ui/toggle"
import type { CatalogSearchHit } from "@/lib/ipc/metadata"
import type { CatalogAddress, CatalogNode, PrivacyTier } from "@/lib/ipc/types"
import { cn } from "@/lib/utils"

/**
 * What decides whether an object can be pinned to the assistant's question.
 *
 * A pin exists to offer a row sample, and a sample leaves only under
 * `sampled`, to the destination the assistant would ask — a built-in provider
 * or an external agent (docs/AI-PROVIDERS.md, « Échantillon approuvé »,
 * ADR-0034). Anywhere else the action is absent rather than disabled: a
 * greyed item would advertise a path the tier closes.
 */
export interface PinToQuestion {
  /** The connection's tier (ADR-0006). */
  tier: PrivacyTier
  /** What the assistant would ask; `null` when it has no destination. */
  destination: "provider" | "agent" | null
  onPin: (node: CatalogNode) => void
}

export function canPin(pin: PinToQuestion | undefined, node: CatalogNode) {
  return (
    pin !== undefined &&
    pin.tier === "sampled" &&
    pin.destination !== null &&
    node.address.relation !== null &&
    node.holdsRecords
  )
}

export function addressKey(address: CatalogAddress) {
  // JSON, not a dotted join: `a.b` in one segment is not `a` then `b` (I-10).
  return JSON.stringify([address.catalog, address.namespace, address.relation])
}

/**
 * An address written to be **read**, never run: segments joined by dots, no
 * quoting. A name to paste into SQL comes quoted from the backend
 * (`qualifiedName`), because this one is ambiguous when a segment has a dot.
 */
export function addressLabel(address: CatalogAddress) {
  return [address.catalog, address.namespace, address.relation]
    .filter((segment): segment is string => segment !== null)
    .join(".")
}

function iconFor(kind: string) {
  switch (kind) {
    case "catalog":
      return DatabaseIcon
    case "namespace":
      return FolderLibraryIcon
    case "view":
    case "materialized_view":
      return EyeIcon
    case "function":
    case "procedure":
      return FunctionIcon
    default:
      return TableIcon
  }
}

export interface Row {
  node: CatalogNode
  depth: number
  key: string
  /** The key of the visible parent row, `null` at the top level. */
  parentKey: string | null
  expandable: boolean
  expanded: boolean
  /** 1-based position among the visible siblings, for `aria-posinset`. */
  posInSet: number
  setSize: number
  /**
   * Set on the row that stands for the contents of an open level showing no
   * child; `node` is then that level. `unloaded`: its contents are not in
   * memory — never read, a failed read, or evicted to keep the catalog cache
   * bounded (docs/ARCHITECTURE.md). `empty`: it was read and holds nothing.
   * `hidden`: it holds only system objects, and they are hidden. Without this
   * row all three look alike, and an evicted schema reads as empty.
   */
  placeholder?: "unloaded" | "empty" | "hidden"
}

/** Flattens the visible part of the tree. Pure, so it is tested without a DOM. */
export function visibleRows(
  nodes: Array<CatalogNode>,
  expanded: ReadonlySet<string>,
  filter: string,
  depth = 0,
  hideSystem = false,
  parentKey: string | null = null
): Array<Row> {
  const needle = filter.trim().toLowerCase()
  const rows: Array<Row> = []
  const siblings: Array<Row> = []
  for (const node of nodes) {
    if (hideSystem && node.system) continue
    const key = addressKey(node.address)
    const expandable = node.kind === "catalog" || node.kind === "namespace"
    if (needle) {
      const childRows = visibleRows(
        node.children,
        expanded,
        filter,
        depth + 1,
        hideSystem,
        key
      )
      const matches = node.name.toLowerCase().includes(needle)
      if (matches || childRows.length > 0) {
        const row: Row = {
          node,
          depth,
          key,
          parentKey,
          expandable,
          expanded: childRows.length > 0,
          posInSet: 0,
          setSize: 0,
        }
        siblings.push(row)
        rows.push(row, ...childRows)
      }
      continue
    }
    const isOpen = expanded.has(key)
    const row: Row = {
      node,
      depth,
      key,
      parentKey,
      expandable,
      expanded: isOpen,
      posInSet: 0,
      setSize: 0,
    }
    siblings.push(row)
    rows.push(row)
    if (!isOpen) continue
    const childRows = visibleRows(
      node.children,
      expanded,
      filter,
      depth + 1,
      hideSystem,
      key
    )
    if (expandable && childRows.length === 0) {
      rows.push({
        node,
        depth: depth + 1,
        key: `${key}:contents`,
        parentKey: key,
        expandable: false,
        expanded: false,
        posInSet: 1,
        setSize: 1,
        placeholder:
          node.children.length > 0
            ? "hidden"
            : node.loaded
              ? "empty"
              : "unloaded",
      })
      continue
    }
    rows.push(...childRows)
  }
  siblings.forEach((row, index) => {
    row.posInSet = index + 1
    row.setSize = siblings.length
  })
  return rows
}

/**
 * The row keyboard focus lands on after `rows` changed: the same key when it
 * is still visible, else the nearest index, bounded — Enter must never act on
 * a row that a filter or a collapse removed. Pure, so it is tested.
 */
export function boundedFocus(
  rows: ReadonlyArray<Pick<Row, "key">>,
  key: string | null,
  lastIndex: number
): number {
  if (rows.length === 0) return -1
  const found = key === null ? -1 : rows.findIndex((row) => row.key === key)
  if (found >= 0) return found
  return Math.max(0, Math.min(lastIndex, rows.length - 1))
}

/**
 * Where typing `buffer` moves focus: the next visible row, after `from`,
 * whose name starts with it, wrapping around. Repeating one letter cycles
 * through the names that start with it. Pure, so it is tested.
 */
export function typeaheadMatch(
  rows: ReadonlyArray<Pick<Row, "node" | "placeholder">>,
  buffer: string,
  from: number
): number {
  if (rows.length === 0 || buffer === "") return -1
  const needle = buffer.toLocaleLowerCase()
  const repeated = [...needle].every((char) => char === needle[0])
  // A fresh prefix may match the row already focused; a repeated letter moves on.
  const start = repeated ? from + 1 : Math.max(from, 0)
  const prefix = repeated ? (needle[0] ?? "") : needle
  for (let step = 0; step < rows.length; step++) {
    const index = (start + step) % rows.length
    const row = rows[index]
    // A contents row carries its level's name, which is not its own.
    if (!row || row.placeholder) continue
    if (row.node.name.toLocaleLowerCase().startsWith(prefix)) return index
  }
  return -1
}

/** The keys of expanded levels a DDL invalidated: what to read again (ADR-0022). */
export function staleExpanded(
  nodes: Array<CatalogNode>,
  expanded: ReadonlySet<string>
): Array<CatalogAddress> {
  const stale: Array<CatalogAddress> = []
  for (const node of nodes) {
    const key = addressKey(node.address)
    if (!expanded.has(key)) continue
    if (node.stale) stale.push(node.address)
    stale.push(...staleExpanded(node.children, expanded))
  }
  return stale
}

/** A tree row: dense, as the sidebar menu rows. */
const ROW_HEIGHT = 28

/** How long typed letters accumulate into one typeahead prefix. */
const TYPEAHEAD_MS = 500

/** Where the object view should open on a relation. */
export type OpenTarget = "data" | "structure" | "definition"

/** What the contents row of an open level says, when it shows no child. */
const PLACEHOLDER: Record<
  NonNullable<Row["placeholder"]>,
  { label: string; title: string }
> = {
  unloaded: {
    label: "Not loaded — click to read",
    title:
      "The contents of this level are not in memory: never read, a read that failed, or unloaded to keep the catalog cache bounded. Click or press Enter to read them.",
  },
  empty: { label: "No objects", title: "This level was read and is empty." },
  hidden: {
    label: "Only system objects (hidden)",
    title: "This level holds only system objects, and they are hidden.",
  },
}

const MATCHED: Record<CatalogSearchHit["matched"], string> = {
  relationName: "name",
  fieldName: "column",
  comment: "comment",
  other: "match",
}

/**
 * The objects of the open connection.
 *
 * Expanding a node asks for that level through the command bus; nothing is
 * invented while it loads (docs/UX-SPEC.md, « Navigation du premier
 * workspace »). The search covers loaded objects only, and says so. System
 * schemas can be hidden; they are never removed from the catalog.
 *
 * Names are text. The context menu copies the name the **backend** quoted
 * for the session's dialect — this component never builds one.
 */
export function CatalogTree({
  nodes,
  loading,
  selected,
  onExpand,
  onSelect,
  expanded: controlledExpanded,
  onExpandedChange,
  search,
  onOpen,
  onCopyName,
  onRefresh,
  pin,
}: {
  nodes: Array<CatalogNode>
  loading: ReadonlySet<string>
  selected: string | null
  onExpand: (address: CatalogAddress) => void
  onSelect: (node: CatalogNode) => void
  /** The expanded levels, when the parent needs them to refresh stale ones. */
  expanded?: ReadonlySet<string>
  onExpandedChange?: (expanded: Set<string>) => void
  /**
   * The catalog search, when the parent provides one: it also matches columns
   * and comments already read. Without it, the filter matches loaded names.
   */
  search?: {
    hits: Array<CatalogSearchHit> | null
    searching: boolean
    onQuery: (query: string) => void
  }
  onOpen?: (node: CatalogNode, target: OpenTarget) => void
  onCopyName?: (node: CatalogNode) => void
  onRefresh?: (node: CatalogNode) => void
  pin?: PinToQuestion
}) {
  const [ownExpanded, setOwnExpanded] = React.useState<Set<string>>(
    () => new Set()
  )
  const expanded = controlledExpanded ?? ownExpanded
  const setExpanded = (update: (current: Set<string>) => Set<string>) => {
    const next = update(new Set(expanded))
    if (onExpandedChange) onExpandedChange(next)
    else setOwnExpanded(next)
  }
  const [filter, setFilter] = React.useState("")
  const [hideSystem, setHideSystem] = React.useState(false)
  const [debouncedFilter] = useDebouncedValue(filter, { wait: 120 })
  const searching = search !== undefined && debouncedFilter.trim() !== ""
  const rows = React.useMemo(
    () =>
      visibleRows(
        nodes,
        expanded,
        searching ? "" : debouncedFilter,
        0,
        hideSystem
      ),
    [nodes, expanded, debouncedFilter, hideSystem, searching]
  )

  // Focus follows a key, not an index: rows come and go under it (a filter,
  // a collapse, a level read again), and the index is re-derived and bounded.
  const [focusKey, setFocusKey] = React.useState<string | null>(null)
  const lastIndex = React.useRef(0)
  const focus = boundedFocus(rows, focusKey, lastIndex.current)
  lastIndex.current = Math.max(focus, 0)
  const focusRow = focus >= 0 ? rows[focus] : undefined
  const focusAt = (index: number) => {
    const row = rows[index]
    if (row) setFocusKey(row.key)
  }

  const [menuNode, setMenuNode] = React.useState<CatalogNode | null>(null)
  const listRef = React.useRef<HTMLDivElement>(null)
  const onQuery = search?.onQuery
  const baseId = React.useId()
  const ids = React.useRef(new Map<string, string>())
  const idFor = (key: string) => {
    let id = ids.current.get(key)
    if (!id) {
      id = `${baseId}-item-${ids.current.size}`
      ids.current.set(key, id)
    }
    return id
  }
  const typed = React.useRef({ buffer: "", at: 0 })

  const virtualizer = useVirtualizer({
    count: rows.length,
    getScrollElement: () => listRef.current,
    estimateSize: () => ROW_HEIGHT,
    overscan: 12,
  })

  React.useEffect(() => {
    if (onQuery && debouncedFilter.trim() !== "") onQuery(debouncedFilter)
  }, [debouncedFilter, onQuery])

  React.useEffect(() => {
    if (focus >= 0) virtualizer.scrollToIndex(focus, { align: "auto" })
    // Scrolled when the focused index moves, not on every render.
  }, [focus])

  const toggle = (row: Row) => {
    if (!row.expandable) return
    const opening = !expanded.has(row.key)
    setExpanded((next) => {
      if (opening) next.add(row.key)
      else next.delete(row.key)
      return next
    })
    if (opening && (!row.node.loaded || row.node.stale))
      onExpand(row.node.address)
  }

  const activate = (row: Row) => {
    // Reading an unloaded level is asked for, never automatic: two open
    // schemas that do not fit the cache together would evict each other.
    if (row.placeholder === "unloaded") {
      if (!loading.has(row.parentKey ?? "")) onExpand(row.node.address)
    } else if (row.placeholder === undefined) {
      if (row.expandable) toggle(row)
      else onSelect(row.node)
    }
  }

  const onKeyDown = (event: React.KeyboardEvent) => {
    const row = focusRow
    const index = Math.max(focus, 0)
    switch (event.key) {
      case "ArrowDown":
        focusAt(Math.min(rows.length - 1, index + 1))
        break
      case "ArrowUp":
        focusAt(Math.max(0, index - 1))
        break
      case "Home":
        focusAt(0)
        break
      case "End":
        focusAt(rows.length - 1)
        break
      case "ArrowRight":
        if (!row) return
        if (row.expandable && !row.expanded) toggle(row)
        else if (row.expanded) focusAt(Math.min(rows.length - 1, index + 1))
        break
      case "ArrowLeft":
        if (!row) return
        if (row.expandable && row.expanded) toggle(row)
        else if (row.parentKey !== null) setFocusKey(row.parentKey)
        break
      case "Enter":
        if (!row) return
        activate(row)
        break
      case " ":
        if (
          typed.current.buffer !== "" &&
          event.timeStamp - typed.current.at < TYPEAHEAD_MS
        ) {
          typeahead(" ", event.timeStamp)
          break
        }
        if (!row) return
        activate(row)
        break
      default:
        if (
          event.key.length === 1 &&
          !event.metaKey &&
          !event.ctrlKey &&
          !event.altKey
        ) {
          typeahead(event.key, event.timeStamp)
          break
        }
        return
    }
    event.preventDefault()
  }

  const typeahead = (char: string, at: number) => {
    const fresh = at - typed.current.at >= TYPEAHEAD_MS
    typed.current = { buffer: (fresh ? "" : typed.current.buffer) + char, at }
    const match = typeaheadMatch(rows, typed.current.buffer, focus)
    if (match >= 0) focusAt(match)
  }

  const hasSystem = nodes.some((node) => node.system)
  const menuRelation = menuNode !== null && menuNode.address.relation !== null

  return (
    <div className="flex min-h-0 flex-1 flex-col gap-2">
      <InputGroup className="h-8">
        <InputGroupAddon>
          {searching && search.searching ? (
            <Spinner />
          ) : (
            <HugeiconsIcon icon={Search01Icon} strokeWidth={2} />
          )}
        </InputGroupAddon>
        <InputGroupInput
          value={filter}
          onChange={(event) => setFilter(event.target.value)}
          placeholder="Filter loaded objects"
          aria-label="Filter loaded objects"
        />
        {hasSystem ? (
          <InputGroupAddon align="inline-end">
            {/* A fixed name and a pressed state: a name that flips as well
                is announced « Show system objects, pressed » — the opposite
                of what the tree shows. */}
            <Toggle
              size="sm"
              pressed={hideSystem}
              onPressedChange={setHideSystem}
              aria-label="Hide system objects"
              title="Hide system objects"
              className="size-6 min-w-6 rounded-[calc(var(--radius)-3px)] px-0"
            >
              <HugeiconsIcon icon={ViewOffIcon} strokeWidth={2} />
            </Toggle>
          </InputGroupAddon>
        ) : null}
      </InputGroup>

      {searching ? (
        <SearchHits
          hits={search.hits}
          searching={search.searching}
          selected={selected}
          onSelect={(hit) =>
            onSelect({
              address: hit.address,
              name: hit.name,
              kind: hit.kind,
              holdsRecords: hit.holdsRecords,
              system: false,
              comment: null,
              loaded: true,
              stale: false,
              children: [],
            })
          }
        />
      ) : rows.length === 0 ? (
        debouncedFilter ? (
          <Empty className="flex-none p-4">
            <EmptyHeader>
              <EmptyTitle>No loaded object matches</EmptyTitle>
            </EmptyHeader>
          </Empty>
        ) : (
          <p className="px-2 py-4 text-xs text-muted-foreground">
            No object loaded yet.
          </p>
        )
      ) : (
        <ContextMenu>
          <ContextMenuTrigger
            render={
              <div
                ref={listRef}
                role="tree"
                aria-label="Catalog"
                aria-activedescendant={
                  // Only a row the virtualizer has drawn exists in the DOM: an
                  // id pointing at nothing is announced as nothing, and the
                  // first frame of a tree just opened has no rows yet.
                  focusRow &&
                  virtualizer
                    .getVirtualItems()
                    .some((item) => item.index === focus)
                    ? idFor(focusRow.key)
                    : undefined
                }
                tabIndex={0}
                onKeyDown={onKeyDown}
                onContextMenu={(event) => {
                  // The menu key opens the menu on the tree itself: it acts
                  // on the focused row, as a right click acts on its row.
                  if (event.target === event.currentTarget && focusRow)
                    setMenuNode(focusRow.node)
                }}
                className="group/tree relative min-h-0 flex-1 overflow-auto rounded-md outline-none"
              />
            }
          >
            <div
              role="none"
              className="relative w-full"
              style={{ height: virtualizer.getTotalSize() }}
            >
              {virtualizer.getVirtualItems().map((item) => {
                const row = rows[item.index]
                if (!row) return null
                const isLoading = loading.has(
                  row.placeholder ? (row.parentKey ?? "") : row.key
                )
                const isFocused = item.index === focus
                const isSelected = selected === row.key
                return (
                  <div
                    key={row.key}
                    id={idFor(row.key)}
                    role="treeitem"
                    data-index={item.index}
                    data-focused={isFocused || undefined}
                    aria-level={row.depth + 1}
                    aria-posinset={row.posInSet}
                    aria-setsize={row.setSize}
                    aria-expanded={row.expandable ? row.expanded : undefined}
                    aria-selected={isSelected}
                    aria-busy={isLoading || undefined}
                    onClick={() => {
                      setFocusKey(row.key)
                      activate(row)
                    }}
                    onContextMenu={() => {
                      setFocusKey(row.key)
                      setMenuNode(row.node)
                    }}
                    title={
                      row.placeholder
                        ? PLACEHOLDER[row.placeholder].title
                        : row.node.comment
                          ? `${row.node.name} — ${row.node.comment}`
                          : row.node.name
                    }
                    className={cn(
                      "absolute inset-x-0 top-0 flex items-center gap-1.5 rounded-md pr-2 text-[length:var(--reading-text)] text-sidebar-foreground hover:bg-sidebar-accent hover:text-sidebar-accent-foreground",
                      row.placeholder && "text-muted-foreground",
                      isSelected &&
                        "bg-sidebar-accent font-medium text-sidebar-accent-foreground",
                      // The ring sits on the row, and only while the tree has
                      // keyboard focus: one ring, never two.
                      isFocused &&
                        "group-focus-visible/tree:ring-2 group-focus-visible/tree:ring-ring group-focus-visible/tree:ring-inset"
                    )}
                    style={{
                      height: ROW_HEIGHT,
                      transform: `translateY(${item.start}px)`,
                      paddingInlineStart: 4 + row.depth * 14,
                    }}
                  >
                    {row.placeholder ? (
                      <span className="min-w-0 truncate ps-5.5 text-[length:var(--reading-caption)]">
                        {isLoading
                          ? "Reading…"
                          : PLACEHOLDER[row.placeholder].label}
                      </span>
                    ) : (
                      <>
                        <span className="flex size-4 shrink-0 items-center justify-center text-muted-foreground">
                          {row.expandable ? (
                            isLoading ? (
                              <Spinner aria-hidden className="size-3" />
                            ) : (
                              <HugeiconsIcon
                                icon={ArrowRight01Icon}
                                strokeWidth={2}
                                className={cn(
                                  "size-3.5 motion-safe:transition-transform rtl:-scale-x-100",
                                  row.expanded && "rotate-90 rtl:-rotate-90"
                                )}
                              />
                            )
                          ) : null}
                        </span>
                        <HugeiconsIcon
                          icon={iconFor(row.node.kind)}
                          strokeWidth={1.8}
                          className="size-4 shrink-0 text-muted-foreground"
                        />
                        <span dir="auto" className="min-w-0 truncate">
                          {row.node.name}
                        </span>
                        {row.node.stale ? (
                          <Badge
                            variant="outline"
                            className="ms-auto h-4 shrink-0 px-1 text-[length:var(--reading-caption)] text-warning"
                          >
                            stale
                          </Badge>
                        ) : row.node.system ? (
                          <span className="ms-auto shrink-0 text-[length:var(--reading-caption)] text-muted-foreground">
                            system
                          </span>
                        ) : null}
                      </>
                    )}
                  </div>
                )
              })}
            </div>
          </ContextMenuTrigger>
          <ContextMenuContent>
            {menuNode ? (
              <>
                <ContextMenuGroup>
                  <ContextMenuLabel dir="auto" className="max-w-64 truncate">
                    {menuNode.name}
                  </ContextMenuLabel>
                  {menuRelation && onOpen ? (
                    <>
                      <ContextMenuItem
                        disabled={!menuNode.holdsRecords}
                        onClick={() => onOpen(menuNode, "data")}
                      >
                        Open data
                      </ContextMenuItem>
                      <ContextMenuItem
                        onClick={() => onOpen(menuNode, "structure")}
                      >
                        View structure
                      </ContextMenuItem>
                      <ContextMenuItem
                        onClick={() => onOpen(menuNode, "definition")}
                      >
                        View DDL
                      </ContextMenuItem>
                    </>
                  ) : null}
                </ContextMenuGroup>
                {(menuRelation && onCopyName) ||
                (!menuRelation && onRefresh) ||
                canPin(pin, menuNode) ? (
                  <ContextMenuSeparator />
                ) : null}
                <ContextMenuGroup>
                  {menuRelation && onCopyName ? (
                    <ContextMenuItem onClick={() => onCopyName(menuNode)}>
                      Copy qualified name
                    </ContextMenuItem>
                  ) : null}
                  {!menuRelation && onRefresh ? (
                    <ContextMenuItem onClick={() => onRefresh(menuNode)}>
                      Refresh this level
                    </ContextMenuItem>
                  ) : null}
                  {pin && canPin(pin, menuNode) ? (
                    <ContextMenuItem onClick={() => pin.onPin(menuNode)}>
                      Pin to question
                    </ContextMenuItem>
                  ) : null}
                </ContextMenuGroup>
              </>
            ) : null}
          </ContextMenuContent>
        </ContextMenu>
      )}
    </div>
  )
}

function SearchHits({
  hits,
  searching,
  selected,
  onSelect,
}: {
  hits: Array<CatalogSearchHit> | null
  searching: boolean
  selected: string | null
  onSelect: (hit: CatalogSearchHit) => void
}) {
  if (hits === null) {
    return searching ? (
      <p role="status" className="px-2 py-4 text-xs text-muted-foreground">
        Searching loaded objects…
      </p>
    ) : null
  }
  if (hits.length === 0) {
    return (
      <Empty className="flex-none p-4">
        <EmptyHeader>
          <EmptyTitle>No loaded object matches</EmptyTitle>
          <EmptyDescription className="text-xs">
            The search only covers what has already been read.
          </EmptyDescription>
        </EmptyHeader>
      </Empty>
    )
  }
  return (
    <ul
      aria-label="Matching objects"
      className="flex min-h-0 flex-1 flex-col overflow-auto"
    >
      {hits.map((hit) => {
        const key = addressKey(hit.address)
        const parent = addressLabel({ ...hit.address, relation: null })
        return (
          <li key={key}>
            <button
              type="button"
              onClick={() => onSelect(hit)}
              aria-current={selected === key || undefined}
              className={cn(
                "flex w-full min-w-0 items-center gap-1.5 rounded-md px-1.5 py-1 text-left text-[length:var(--reading-text)] outline-none hover:bg-sidebar-accent focus-visible:ring-2 focus-visible:ring-ring",
                selected === key && "bg-sidebar-accent font-medium"
              )}
            >
              <HugeiconsIcon
                icon={iconFor(hit.kind)}
                strokeWidth={1.8}
                className="size-4 shrink-0 text-muted-foreground"
              />
              <span className="flex min-w-0 flex-col">
                <span dir="auto" className="truncate">
                  {hit.name}
                </span>
                <span className="truncate text-[length:var(--reading-caption)] text-muted-foreground">
                  {parent}
                  {hit.matched === "fieldName" && hit.matchedFields.length > 0
                    ? ` · column ${hit.matchedFields.join(", ")}`
                    : ` · ${MATCHED[hit.matched]}`}
                </span>
              </span>
            </button>
          </li>
        )
      })}
    </ul>
  )
}
