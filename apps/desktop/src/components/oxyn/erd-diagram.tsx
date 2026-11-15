import { ContentHint } from "@/components/oxyn/content-hint"
import * as React from "react"
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

import { ActionMenuContent } from "@/components/oxyn/action-menu-items"
import { NODE_WIDTH, layoutErd, retainErdPositions } from "./erd-layout"
import { visibleColumns } from "./erd-model"
import { ERD_MAX_TABLES } from "@/components/oxyn/erd-model"
import type { ErdLink, ErdTable } from "@/components/oxyn/erd-model"
import { Button } from "@/components/ui/button"
import { Badge } from "@/components/ui/badge"
import { ContextMenu, ContextMenuTrigger } from "@/components/ui/context-menu"
import { cn } from "@/lib/utils"
import type { CatalogAddress } from "@/lib/ipc/types"

export { layoutErd } from "./erd-layout"
export { visibleColumns } from "./erd-model"
export { ERD_MAX_COLUMNS, ERD_MAX_TABLES } from "@/components/oxyn/erd-model"
export type { ErdColumn, ErdLink, ErdTable } from "@/components/oxyn/erd-model"

function tableLabel(table: Pick<ErdTable, "name" | "namespace" | "catalog">) {
  return [table.catalog, table.namespace, table.name]
    .filter((part) => part !== null && part !== undefined)
    .join(".")
}

type TableNodeData = {
  table: ErdTable
  onOpen?: (address: CatalogAddress) => void
  onCopyName?: (table: ErdTable) => void
  actionLabel?: string
}
type TableNode = Node<TableNodeData, "table">

/** A table, drawn as React text: its name and columns come from a server. */
function TableNodeView({ data }: NodeProps<TableNode>) {
  const { table, onOpen, onCopyName, actionLabel = "Open" } = data
  const { shown, hidden } = visibleColumns(table.columns)
  const label = tableLabel(table)
  const [anchor, setAnchor] = React.useState<Element | null>(null)
  const title = (
    <>
      <HugeiconsIcon
        icon={Table01Icon}
        strokeWidth={2}
        className="size-3.5 shrink-0 text-muted-foreground"
        aria-hidden
      />
      <span className="min-w-0 truncate">
        {table.catalog && (
          <span className="text-muted-foreground">{table.catalog}.</span>
        )}
        {table.namespace !== null ? (
          <span className="text-muted-foreground">{table.namespace}.</span>
        ) : null}
        <span className="font-medium">{table.name}</span>
        {table.kind && (
          <Badge variant="outline">{table.kind.replaceAll("_", " ")}</Badge>
        )}
      </span>
    </>
  )
  return (
    <ContextMenu>
      <ContextMenuTrigger
        render={
          <div
            data-slot="erd-table"
            data-requested={table.requested || undefined}
            className={cn(
              // The canvas turns pointer events off on a node that is neither
              // selectable nor draggable; its button and its menu must still
              // take a click.
              "pointer-events-auto flex flex-col overflow-hidden rounded-lg border bg-card text-xs text-card-foreground shadow-xs",
              table.requested && "border-primary/60"
            )}
            style={{ width: NODE_WIDTH }}
            onContextMenu={(event) => setAnchor(event.target as Element)}
          />
        }
      >
        <Handle
          type="target"
          position={Position.Left}
          isConnectable={false}
          className="size-1! min-h-0! min-w-0! border-0! bg-transparent!"
        />
        {onOpen ? (
          <ContentHint content={label}>
            <Button
              variant="ghost"
              size="sm"
              aria-label={`${actionLabel} ${label}`}
              className="nodrag nopan h-8 w-full justify-start gap-1.5 rounded-none border-b px-2 text-xs"
              onClick={() => onOpen(table.address)}
            >
              {title}
            </Button>
          </ContentHint>
        ) : (
          <ContentHint content={label}>
            <div className="flex h-8 items-center gap-1.5 border-b px-2">
              {title}
            </div>
          </ContentHint>
        )}
        {table.fieldsState === "stale" && (
          <Badge variant="outline">Retained stale fields</Badge>
        )}
        <ul className="flex flex-col py-1">
          {shown.length === 0 ? (
            <li className="flex h-5.5 items-center px-2 text-muted-foreground">
              {table.columnsKnown === false
                ? "Columns not loaded"
                : "No columns reported"}
            </li>
          ) : null}
          {shown.map((column) => (
            <li
              key={column.key ?? column.name}
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
              <ContentHint content={column.name}>
                <span className="min-w-0 truncate font-mono">
                  {column.name}
                </span>
              </ContentHint>
              {column.primaryKey ? (
                <span className="sr-only">, primary key</span>
              ) : null}
              {column.foreignKey ? (
                <span className="sr-only">, foreign key</span>
              ) : null}
              {column.origin && column.origin !== "reported" && (
                <Badge variant="outline">
                  {column.origin === "sampled" ? "Sampled" : "Layout"}
                </Badge>
              )}
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
      </ContextMenuTrigger>
      <ActionMenuContent
        surface="erd"
        anchor={anchor}
        title={label}
        sources={{
          erd: {
            state: {},
            actions: {
              openTable: onOpen ? () => onOpen(table.address) : undefined,
              copyName: onCopyName ? () => onCopyName(table) : undefined,
            },
          },
        }}
      />
    </ContextMenu>
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
  onCopyName,
  visibleKeys,
  actionLabel,
}: {
  tables: Array<ErdTable>
  links: Array<ErdLink>
  onOpenObject?: (address: CatalogAddress) => void
  onCopyName?: (table: ErdTable) => void
  visibleKeys?: ReadonlySet<string>
  actionLabel?: string
}) {
  const positionsRef = React.useRef(new Map<string, { x: number; y: number }>())
  const flow = useReactFlow()
  const keysHint = React.useId()
  const positions = React.useMemo(() => {
    const arranged = layoutErd(tables, links)
    const next = retainErdPositions(tables, arranged, positionsRef.current)
    positionsRef.current = next
    return next
  }, [tables, links])
  const { nodes, edges } = React.useMemo(() => {
    const byKey = new Map(tables.map((table) => [table.key, table]))
    return {
      nodes: tables.map((table): TableNode => ({
        id: table.key,
        type: "table",
        position: positions.get(table.key) ?? { x: 0, y: 0 },
        data: { table, onOpen: onOpenObject, onCopyName, actionLabel },
        hidden: visibleKeys !== undefined && !visibleKeys.has(table.key),
      })),
      edges: links.flatMap((link): Array<Edge> => {
        const from = byKey.get(link.from)
        const to = byKey.get(link.to)
        if (!from || !to) return []
        return [
          {
            id: link.key,
            hidden:
              visibleKeys !== undefined &&
              (!visibleKeys.has(link.from) || !visibleKeys.has(link.to)),
            // The default label spells the internal keys.
            ariaLabel: `${link.name}: ${tableLabel(from)} references ${tableLabel(to)}`,
            source: link.from,
            target: link.to,
            type: "smoothstep",
            markerEnd: {
              type: MarkerType.ArrowClosed,
              color: "var(--muted-foreground)",
            },
            style: {
              stroke: "var(--muted-foreground)",
              strokeWidth: 1.25,
              strokeDasharray: link.stale ? "4 3" : undefined,
            },
          },
        ]
      }),
    }
  }, [
    tables,
    links,
    positions,
    onOpenObject,
    onCopyName,
    visibleKeys,
    actionLabel,
  ])

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
      aria-label={`Diagram of ${tables.length} ${tables.some((table) => table.kind) ? "objects" : tables.length === 1 ? "table" : "tables"}`}
      aria-describedby={keysHint}
      onKeyDown={onKeyDown}
      // Its pinch zooms the diagram, which the page's zoom guard leaves to it.
      data-own-zoom=""
      className="h-90 min-h-80 min-w-0 flex-1 outline-none focus-visible:ring-2 focus-visible:ring-ring focus-visible:ring-inset"
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
 * opens it, as the catalog would; nothing runs. A right click on a table
 * offers `Open table` and `Copy name`; `Re-layout` and `Export image…` are
 * greyed with what is missing.
 */
export function ErdDiagram({
  tables,
  links,
  omitted = 0,
  onOpenObject,
  onCopyName,
  visibleKeys,
  actionLabel,
  className,
}: {
  tables: Array<ErdTable>
  links: Array<ErdLink>
  /** Tables related to those drawn, left out by `ERD_MAX_TABLES`. */
  omitted?: number
  onOpenObject?: (address: CatalogAddress) => void
  /** Copies the table's name — the one the backend quotes, where it can. */
  onCopyName?: (table: ErdTable) => void
  visibleKeys?: ReadonlySet<string>
  actionLabel?: string
  className?: string
}) {
  const byKey = new Map(tables.map((table) => [table.key, table]))
  const drawnLinks = links.filter(
    (link) =>
      byKey.has(link.from) &&
      byKey.has(link.to) &&
      (visibleKeys === undefined ||
        (visibleKeys.has(link.from) && visibleKeys.has(link.to)))
  )
  return (
    <div
      data-slot="erd-diagram"
      className={cn("flex min-w-0 flex-col", className)}
    >
      <ReactFlowProvider>
        <Canvas
          tables={tables}
          links={links}
          onOpenObject={onOpenObject}
          onCopyName={onCopyName}
          visibleKeys={visibleKeys}
          actionLabel={actionLabel}
        />
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
          {visibleKeys?.size ?? tables.length}{" "}
          {tables.some((table) => table.kind)
            ? "objects"
            : tables.length === 1
              ? "table"
              : "tables"}
          , {drawnLinks.length} {drawnLinks.length === 1 ? "key" : "keys"}
        </span>
        {omitted > 0 ? (
          <span>
            · {omitted} more related{" "}
            {tables.some((table) => table.kind)
              ? "objects"
              : omitted === 1
                ? "table"
                : "tables"}{" "}
            not drawn: a diagram stops at {ERD_MAX_TABLES}
          </span>
        ) : null}
      </p>
    </div>
  )
}
