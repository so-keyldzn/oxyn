import * as React from "react"
import dagre from "@dagrejs/dagre"
import {
  Background,
  BackgroundVariant,
  Handle,
  MarkerType,
  Panel,
  Position,
  ReactFlow,
  ReactFlowProvider,
  useReactFlow,
} from "@xyflow/react"
import type { Edge, Node, NodeProps } from "@xyflow/react"
import { HugeiconsIcon } from "@hugeicons/react"
import {
  FitToScreenIcon,
  Key01Icon,
  Link01Icon,
  Table01Icon,
  ZoomInAreaIcon,
  ZoomOutAreaIcon,
} from "@hugeicons/core-free-icons"
import "@xyflow/react/dist/base.css"

import { ERD_MAX_COLUMNS, ERD_MAX_TABLES } from "@/components/oxyn/erd-model"
import type { ErdColumn, ErdLink, ErdTable } from "@/components/oxyn/erd-model"
import { Button } from "@/components/ui/button"
import { cn } from "@/lib/utils"
import type { CatalogAddress } from "@/lib/ipc/types"

export { ERD_MAX_COLUMNS, ERD_MAX_TABLES } from "@/components/oxyn/erd-model"
export type { ErdColumn, ErdLink, ErdTable } from "@/components/oxyn/erd-model"

const NODE_WIDTH = 240
const HEADER_HEIGHT = 32
const ROW_HEIGHT = 22
const NODE_PADDING = 8

/** The columns drawn: keys first, in position order, bounded. Pure, so tested. */
export function visibleColumns(columns: Array<ErdColumn>) {
  const keys = columns.filter(
    (column) => column.primaryKey || column.foreignKey
  )
  const rest = columns.filter(
    (column) => !column.primaryKey && !column.foreignKey
  )
  const shown = [...keys, ...rest].slice(0, ERD_MAX_COLUMNS)
  return { shown, hidden: columns.length - shown.length }
}

function nodeHeight(table: ErdTable) {
  const { shown, hidden } = visibleColumns(table.columns)
  const rows = Math.max(shown.length, 1) + (hidden > 0 ? 1 : 0)
  return HEADER_HEIGHT + rows * ROW_HEIGHT + NODE_PADDING
}

function tableLabel(table: Pick<ErdTable, "name" | "namespace">) {
  return table.namespace === null
    ? table.name
    : `${table.namespace}.${table.name}`
}

/**
 * Places the tables left to right, referencing table after referenced one's
 * dependants. Pure and deterministic, so the same answer draws the same map.
 */
export function layoutErd(
  tables: Array<ErdTable>,
  links: Array<ErdLink>
): Map<string, { x: number; y: number }> {
  const graph = new dagre.graphlib.Graph({ multigraph: true })
  graph.setGraph({ rankdir: "LR", nodesep: 32, ranksep: 72 })
  graph.setDefaultEdgeLabel(() => ({}))
  for (const table of tables)
    graph.setNode(table.key, { width: NODE_WIDTH, height: nodeHeight(table) })
  for (const link of links)
    if (graph.hasNode(link.from) && graph.hasNode(link.to))
      graph.setEdge(link.from, link.to, {}, link.key)
  dagre.layout(graph)
  const positions = new Map<string, { x: number; y: number }>()
  for (const table of tables) {
    const node = graph.node(table.key)
    // dagre gives centres; the canvas places corners.
    positions.set(table.key, {
      x: (node.x ?? 0) - NODE_WIDTH / 2,
      y: (node.y ?? 0) - nodeHeight(table) / 2,
    })
  }
  return positions
}

type TableNodeData = {
  table: ErdTable
  onOpen?: (address: CatalogAddress) => void
}
type TableNode = Node<TableNodeData, "table">

/** A table, drawn as React text: its name and columns come from a server. */
function TableNodeView({ data }: NodeProps<TableNode>) {
  const { table, onOpen } = data
  const { shown, hidden } = visibleColumns(table.columns)
  const label = tableLabel(table)
  const title = (
    <>
      <HugeiconsIcon
        icon={Table01Icon}
        strokeWidth={2}
        className="size-3.5 shrink-0 text-muted-foreground"
        aria-hidden
      />
      <span className="min-w-0 truncate">
        {table.namespace !== null ? (
          <span className="text-muted-foreground">{table.namespace}.</span>
        ) : null}
        <span className="font-medium">{table.name}</span>
      </span>
    </>
  )
  return (
    <div
      data-slot="erd-table"
      data-requested={table.requested || undefined}
      className={cn(
        // The canvas turns pointer events off on a node that is neither
        // selectable nor draggable; its button must still take a click.
        "pointer-events-auto flex flex-col overflow-hidden rounded-lg border bg-card text-xs text-card-foreground shadow-xs",
        table.requested && "border-primary/60"
      )}
      style={{ width: NODE_WIDTH }}
    >
      <Handle
        type="target"
        position={Position.Left}
        isConnectable={false}
        className="size-1! min-h-0! min-w-0! border-0! bg-transparent!"
      />
      {onOpen ? (
        <Button
          variant="ghost"
          size="sm"
          title={label}
          aria-label={`Open ${label}`}
          className="nodrag nopan h-8 w-full justify-start gap-1.5 rounded-none border-b px-2 text-xs"
          onClick={() => onOpen(table.address)}
        >
          {title}
        </Button>
      ) : (
        <div
          title={label}
          className="flex h-8 items-center gap-1.5 border-b px-2"
        >
          {title}
        </div>
      )}
      <ul className="flex flex-col py-1">
        {shown.length === 0 ? (
          <li className="flex h-5.5 items-center px-2 text-muted-foreground">
            No columns reported
          </li>
        ) : null}
        {shown.map((column) => (
          <li
            key={column.name}
            className="flex h-5.5 min-w-0 items-center gap-1.5 px-2"
          >
            <span className="flex w-3.5 shrink-0 justify-center">
              {column.primaryKey ? (
                <HugeiconsIcon
                  icon={Key01Icon}
                  strokeWidth={2}
                  className="size-3 text-warning"
                  aria-hidden
                />
              ) : column.foreignKey ? (
                <HugeiconsIcon
                  icon={Link01Icon}
                  strokeWidth={2}
                  className="size-3 text-primary-text"
                  aria-hidden
                />
              ) : null}
            </span>
            <span className="min-w-0 truncate font-mono" title={column.name}>
              {column.name}
            </span>
            {column.primaryKey ? (
              <span className="sr-only">, primary key</span>
            ) : null}
            {column.foreignKey ? (
              <span className="sr-only">, foreign key</span>
            ) : null}
            <span className="ml-auto max-w-[45%] shrink-0 truncate pl-2 font-mono text-muted-foreground">
              {column.type}
            </span>
          </li>
        ))}
        {hidden > 0 ? (
          <li className="flex h-5.5 items-center px-2 text-muted-foreground">
            {hidden} more {hidden === 1 ? "column" : "columns"}
          </li>
        ) : null}
      </ul>
      <Handle
        type="source"
        position={Position.Right}
        isConnectable={false}
        className="size-1! min-h-0! min-w-0! border-0! bg-transparent!"
      />
    </div>
  )
}

// Outside the component: a new object per render makes the canvas remount
// every node.
const NODE_TYPES = { table: TableNodeView }

const PAN_STEP = 48

function Controls() {
  const flow = useReactFlow()
  return (
    <Panel position="top-right" className="m-2! flex gap-1">
      <Button
        size="icon-xs"
        variant="outline"
        aria-label="Zoom in"
        onClick={() => void flow.zoomIn()}
      >
        <HugeiconsIcon icon={ZoomInAreaIcon} strokeWidth={2} />
      </Button>
      <Button
        size="icon-xs"
        variant="outline"
        aria-label="Zoom out"
        onClick={() => void flow.zoomOut()}
      >
        <HugeiconsIcon icon={ZoomOutAreaIcon} strokeWidth={2} />
      </Button>
      <Button
        size="icon-xs"
        variant="outline"
        aria-label="Fit to view"
        onClick={() => void flow.fitView({ padding: 0.15, maxZoom: 1 })}
      >
        <HugeiconsIcon icon={FitToScreenIcon} strokeWidth={2} />
      </Button>
    </Panel>
  )
}

function Canvas({
  tables,
  links,
  onOpenObject,
}: {
  tables: Array<ErdTable>
  links: Array<ErdLink>
  onOpenObject?: (address: CatalogAddress) => void
}) {
  const flow = useReactFlow()
  const keysHint = React.useId()
  const { nodes, edges } = React.useMemo(() => {
    const positions = layoutErd(tables, links)
    const byKey = new Map(tables.map((table) => [table.key, table]))
    return {
      nodes: tables.map((table): TableNode => ({
        id: table.key,
        type: "table",
        position: positions.get(table.key) ?? { x: 0, y: 0 },
        data: { table, onOpen: onOpenObject },
      })),
      edges: links.flatMap((link): Array<Edge> => {
        const from = byKey.get(link.from)
        const to = byKey.get(link.to)
        if (!from || !to) return []
        return [
          {
            id: link.key,
            // The default label spells the internal keys.
            ariaLabel: `${link.name}: ${tableLabel(from)} references ${tableLabel(to)}`,
            source: link.from,
            target: link.to,
            type: "smoothstep",
            markerEnd: {
              type: MarkerType.ArrowClosed,
              color: "var(--muted-foreground)",
            },
            style: { stroke: "var(--muted-foreground)", strokeWidth: 1.25 },
          },
        ]
      }),
    }
  }, [tables, links, onOpenObject])

  // The canvas takes arrows and +/- only when it has the focus itself: a
  // button inside keeps its own keys.
  const onKeyDown = (event: React.KeyboardEvent<HTMLDivElement>) => {
    if (event.target !== event.currentTarget) return
    const viewport = flow.getViewport()
    const pan = (x: number, y: number) =>
      void flow.setViewport({
        ...viewport,
        x: viewport.x + x,
        y: viewport.y + y,
      })
    switch (event.key) {
      case "ArrowLeft":
        pan(PAN_STEP, 0)
        break
      case "ArrowRight":
        pan(-PAN_STEP, 0)
        break
      case "ArrowUp":
        pan(0, PAN_STEP)
        break
      case "ArrowDown":
        pan(0, -PAN_STEP)
        break
      case "+":
      case "=":
        void flow.zoomIn()
        break
      case "-":
        void flow.zoomOut()
        break
      case "0":
        void flow.fitView({ padding: 0.15, maxZoom: 1 })
        break
      default:
        return
    }
    event.preventDefault()
  }

  return (
    <div
      role="region"
      tabIndex={0}
      aria-label={`Diagram of ${tables.length} ${tables.length === 1 ? "table" : "tables"}`}
      aria-describedby={keysHint}
      onKeyDown={onKeyDown}
      className="h-90 min-w-0 outline-none focus-visible:ring-2 focus-visible:ring-ring focus-visible:ring-inset"
    >
      <span id={keysHint} hidden>
        Arrow keys move the diagram, plus and minus zoom, zero fits it to the
        view.
      </span>
      <ReactFlow
        nodes={nodes}
        edges={edges}
        nodeTypes={NODE_TYPES}
        fitView
        fitViewOptions={{ padding: 0.15, maxZoom: 1 }}
        minZoom={0.1}
        maxZoom={2}
        nodesDraggable={false}
        nodesConnectable={false}
        nodesFocusable={false}
        edgesFocusable={false}
        elementsSelectable={false}
        // The diagram sits in a scrolling conversation: the wheel scrolls the
        // conversation, zoom is the buttons, the keys and a pinch.
        zoomOnScroll={false}
        panOnScroll={false}
        preventScrolling={false}
        zoomOnDoubleClick={false}
        // The attribution stays: its authors ask that only Pro subscribers
        // remove it (docs/RESEARCH-NOTES.md). Its address is fixed by the
        // library, never chosen by a model. Its own colours (#999 on a white
        // veil) fail contrast in both themes: the app's tokens replace them,
        // with `!` because the library's sheet is outside any layer and beats
        // every utility.
        className="bg-background [&_.react-flow\_\_attribution]:bg-transparent! [&_.react-flow\_\_attribution_a]:text-muted-foreground!"
      >
        <Background
          variant={BackgroundVariant.Dots}
          gap={16}
          size={1}
          color="var(--border)"
        />
        <Controls />
      </ReactFlow>
    </div>
  )
}

/**
 * Tables and their foreign keys, laid out by dagre.
 *
 * Everything shown comes from the catalog, read by the backend: nothing here
 * is drawn from what a model wrote. Names are React text. A click on a table
 * opens it, as the catalog would; nothing runs.
 */
export function ErdDiagram({
  tables,
  links,
  omitted = 0,
  onOpenObject,
  className,
}: {
  tables: Array<ErdTable>
  links: Array<ErdLink>
  /** Tables related to those drawn, left out by `ERD_MAX_TABLES`. */
  omitted?: number
  onOpenObject?: (address: CatalogAddress) => void
  className?: string
}) {
  const byKey = new Map(tables.map((table) => [table.key, table]))
  const drawnLinks = links.filter(
    (link) => byKey.has(link.from) && byKey.has(link.to)
  )
  return (
    <div
      data-slot="erd-diagram"
      className={cn("flex min-w-0 flex-col", className)}
    >
      <ReactFlowProvider>
        <Canvas tables={tables} links={links} onOpenObject={onOpenObject} />
      </ReactFlowProvider>
      {/* The edges are drawn, not written: this list is what a screen
          reader gets of them. */}
      <ul className="sr-only" aria-label="Foreign keys">
        {drawnLinks.map((link) => {
          const from = byKey.get(link.from)
          const to = byKey.get(link.to)
          return from && to ? (
            <li key={link.key}>
              {tableLabel(from)} ({link.fields.join(", ")}) references{" "}
              {tableLabel(to)} ({link.referencedFields.join(", ")})
            </li>
          ) : null
        })}
      </ul>
      <p className="flex flex-wrap gap-x-2 border-t px-3 py-1.5 text-xs text-muted-foreground">
        <span className="tabular-nums">
          {tables.length} {tables.length === 1 ? "table" : "tables"},{" "}
          {drawnLinks.length} {drawnLinks.length === 1 ? "key" : "keys"}
        </span>
        {omitted > 0 ? (
          <span>
            · {omitted} more related {omitted === 1 ? "table" : "tables"} not
            drawn: a diagram stops at {ERD_MAX_TABLES}
          </span>
        ) : null}
      </p>
    </div>
  )
}
