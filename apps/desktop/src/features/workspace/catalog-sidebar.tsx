import * as React from "react"
import { useQuery, useQueryClient } from "@tanstack/react-query"

import { CatalogPanel } from "@/components/oxyn/catalog-panel"
import type { CatalogProblem } from "@/components/oxyn/catalog-panel"
import { addressKey, staleExpanded } from "@/components/oxyn/catalog-tree"
import type { OpenTarget } from "@/components/oxyn/catalog-tree"
import { Sidebar, SidebarRail } from "@/components/ui/sidebar"
import { usePinToQuestion } from "@/features/assistant/object-pin"
import { copyToClipboard } from "@/features/metadata/clipboard"
import { useObjectOperation } from "@/features/metadata/object-operation-flow"
import { useRefreshSignal } from "@/features/metadata/refresh-signals"
import { hasCapability } from "@/features/session"
import { BackendError, backend, newCommandId } from "@/lib/ipc/client"
import { metadata } from "@/lib/ipc/metadata"
import type {
  CatalogAddress,
  CatalogNode,
  OpenConnection,
} from "@/lib/ipc/types"

const CATALOG_CAPABILITIES = ["SCHEMAS", "TABLES", "VIEWS", "ROUTINES"]

function findNode(nodes: Array<CatalogNode>, key: string): CatalogNode | null {
  for (const node of nodes) {
    if (addressKey(node.address) === key) return node
    const found = findNode(node.children, key)
    if (found) return found
  }
  return null
}

function problemOf(caught: unknown): CatalogProblem {
  return {
    kind: "error",
    message: caught instanceof Error ? caught.message : String(caught),
    retryable: caught instanceof BackendError ? caught.retryable : false,
  }
}

/**
 * The catalog sidebar: reads the tree through the command bus and hands it to
 * `CatalogPanel`.
 *
 * One path reads the catalog again after a DDL: the refresh signal
 * (ADR-0022), not the execution event stream as well. Every refresh carries
 * a command id, so the whole-catalog one can be cancelled, and all of them
 * are cancelled when the sidebar unmounts.
 */
export function CatalogSidebar({
  open,
  selected,
  onSelect,
  onOpen,
  onLeave,
}: {
  open: OpenConnection
  selected: string | null
  onSelect: (node: CatalogNode) => void
  /** Opens a relation on a given tab; without it, the menu selects the node. */
  onOpen?: (node: CatalogNode, target: OpenTarget) => void
  onLeave: () => void
}) {
  const queryClient = useQueryClient()
  const pin = usePinToQuestion(open)
  const treeNodes = React.useRef<Array<CatalogNode>>([])
  // After a drop, the selection moves to the parent level; the tree itself
  // changes once the executor's invalidation is read again (ADR-0042).
  const operation = useObjectOperation(open, (node, kind) => {
    if (kind !== "drop") return
    const parent = findNode(
      treeNodes.current,
      addressKey({ ...node.address, relation: null })
    )
    if (parent) onSelect(parent)
  })
  const [loading, setLoading] = React.useState<Set<string>>(() => new Set())
  const [expanded, setExpanded] = React.useState<Set<string>>(() => new Set())
  const [problem, setProblem] = React.useState<CatalogProblem | null>(null)
  const [notice, setNotice] = React.useState<string | null>(null)
  const [query, setQuery] = React.useState("")
  const [rootCommand, setRootCommand] = React.useState<string | null>(null)
  const [cancelling, setCancelling] = React.useState(false)
  const running = React.useRef(new Set<string>())
  // The interface is conditional on capabilities (ADR-0003): a key-value store
  // has no tree to draw, and must not pretend to have an empty one.
  const supported = CATALOG_CAPABILITIES.some((name) =>
    hasCapability(open, name)
  )

  const tree = useQuery({
    queryKey: ["catalog", open.connection],
    queryFn: () => metadata.catalogTree(open.connection),
    enabled: supported,
    placeholderData: (previous) => previous,
  })

  treeNodes.current = tree.data ?? []

  const hits = useQuery({
    queryKey: ["catalog-search", open.connection, query],
    queryFn: () => metadata.searchCatalog(open.connection, query),
    enabled: supported && query.trim() !== "",
    placeholderData: (previous) => previous,
  })

  const refresh = React.useCallback(
    async (address: CatalogAddress | null) => {
      const key = address ? addressKey(address) : "root"
      const id = newCommandId()
      running.current.add(id)
      setLoading((current) => new Set(current).add(key))
      if (!address) {
        setRootCommand(id)
        setCancelling(false)
      }
      setProblem(null)
      setNotice(null)
      try {
        const outcome = await metadata.refreshCatalog(
          id,
          open.connection,
          open.session,
          address
        )
        if (outcome.type === "denied")
          setProblem({ kind: "denied", reason: outcome.reason })
        else if (outcome.type === "cancelled")
          setNotice("Refresh cancelled. The tree shows the last read.")
      } catch (caught) {
        setProblem(problemOf(caught))
      } finally {
        running.current.delete(id)
        if (!address) {
          setRootCommand((current) => (current === id ? null : current))
          setCancelling(false)
        }
        await queryClient.invalidateQueries({
          queryKey: ["catalog", open.connection],
        })
        await queryClient.invalidateQueries({
          queryKey: ["catalog-search", open.connection],
        })
        // Only once the tree read again is shown: released earlier, a level
        // just read would still say « Not loaded » for one round trip.
        setLoading((current) => {
          const next = new Set(current)
          next.delete(key)
          return next
        })
      }
    },
    [open.connection, open.session, queryClient]
  )

  React.useEffect(
    () => () => {
      for (const id of running.current)
        void backend.cancel(id).catch(() => undefined)
      running.current.clear()
    },
    []
  )

  const loadedOnce = React.useRef(false)
  React.useEffect(() => {
    if (!supported || loadedOnce.current) return
    loadedOnce.current = true
    void refresh(null)
  }, [supported, refresh])

  // After a DDL, the cache is invalidated: the root and every level the user
  // has open are read again, and only those (ADR-0022). A signal that arrives
  // while this runs is owed, and read once when it ends.
  const reloading = React.useRef(false)
  const owed = React.useRef(false)
  // Read through a function: a signal can set it during the awaits below.
  const isOwed = () => owed.current
  const expandedRef = React.useRef(expanded)
  expandedRef.current = expanded
  const reloadStale = React.useCallback(async () => {
    if (reloading.current) {
      owed.current = true
      return
    }
    reloading.current = true
    try {
      do {
        owed.current = false
        await refresh(null)
        const nodes = await metadata.catalogTree(open.connection)
        for (const address of staleExpanded(nodes, expandedRef.current)) {
          await refresh(address)
        }
      } while (isOwed())
    } catch (caught) {
      setProblem(problemOf(caught))
    } finally {
      reloading.current = false
    }
  }, [open.connection, refresh])

  useRefreshSignal(open.connection, (signal) => {
    if (
      !supported ||
      (signal.type !== "catalogInvalidated" && signal.type !== "lagged")
    )
      return
    void reloadStale()
  })

  const copyName = async (node: CatalogNode) => {
    try {
      const facets = await metadata.relationFacets(
        open.connection,
        node.address
      )
      await copyToClipboard(facets.qualifiedName, "Qualified name")
    } catch (caught) {
      setProblem(problemOf(caught))
    }
  }

  const cancelRefresh = () => {
    if (!rootCommand || cancelling) return
    setCancelling(true)
    void backend.cancel(rootCommand).catch(() => setCancelling(false))
  }

  return (
    <Sidebar variant="inset" collapsible="icon">
      <CatalogPanel
        connectionName={open.name}
        driver={open.driver}
        supported={supported}
        nodes={tree.data}
        loading={loading}
        selected={selected}
        expanded={expanded}
        onExpandedChange={setExpanded}
        onExpand={(address) => void refresh(address)}
        onSelect={onSelect}
        search={{
          hits: query.trim() === "" ? null : (hits.data ?? null),
          searching: hits.isFetching,
          onQuery: setQuery,
        }}
        refreshing={rootCommand !== null}
        cancelling={cancelling}
        onRefresh={() => void refresh(null)}
        onCancelRefresh={cancelRefresh}
        problem={
          problem ??
          (tree.isError && tree.data === undefined
            ? problemOf(tree.error)
            : null)
        }
        notice={notice}
        onDismissProblem={() => setProblem(null)}
        onOpen={onOpen ?? ((node) => onSelect(node))}
        onCopyName={(node) => void copyName(node)}
        onRefreshLevel={(node) => void refresh(node.address)}
        pin={pin}
        operations={{
          capabilities: open.capabilities,
          onOperation: operation.start,
        }}
        onLeave={onLeave}
      />
      <SidebarRail />
      {operation.element}
    </Sidebar>
  )
}
