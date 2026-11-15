import dagre from "@dagrejs/dagre"
import { visibleColumns } from "./erd-model"
import type { ErdTable, ErdLink } from "./erd-model"

export const NODE_WIDTH = 240
const HEADER_HEIGHT = 32
const ROW_HEIGHT = 22
const NODE_PADDING = 8

export function nodeHeight(table: ErdTable) {
  const { shown, hidden } = visibleColumns(table.columns)
  const rows = Math.max(shown.length, 1) + (hidden > 0 ? 1 : 0)
  return (
    HEADER_HEIGHT +
    rows * ROW_HEIGHT +
    NODE_PADDING +
    (table.fieldsState === "stale" ? 20 : 0)
  )
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

/** Retain unchanged placements; added or taller cards never overlap retained neighbours. */
export function retainErdPositions(
  tables: Array<ErdTable>,
  proposed: Map<string, { x: number; y: number }>,
  previous: Map<string, { x: number; y: number }>
): Map<string, { x: number; y: number }> {
  if (!tables.some((table) => previous.has(table.key))) return proposed
  const positions = new Map<string, { x: number; y: number }>()
  const placed: Array<{ x: number; y: number; height: number }> = []
  const ordered = [...tables].sort((a, b) => {
    const one = previous.get(a.key),
      two = previous.get(b.key)
    if (one && two) return one.y - two.y || one.x - two.x
    return one ? -1 : two ? 1 : 0
  })
  for (const table of ordered) {
    const point = previous.get(table.key) ??
      proposed.get(table.key) ?? { x: 0, y: 0 }
    const x = point.x
    let y = point.y
    const height = nodeHeight(table)
    let collision = placed.find(
      (rect) =>
        x < rect.x + NODE_WIDTH &&
        x + NODE_WIDTH > rect.x &&
        y < rect.y + rect.height &&
        y + height > rect.y
    )
    while (collision) {
      y = collision.y + collision.height + 32
      collision = placed.find(
        (rect) =>
          x < rect.x + NODE_WIDTH &&
          x + NODE_WIDTH > rect.x &&
          y < rect.y + rect.height &&
          y + height > rect.y
      )
    }
    positions.set(table.key, { x, y })
    placed.push({ x, y, height })
  }
  return positions
}
