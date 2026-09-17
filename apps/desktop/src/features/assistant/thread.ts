// A conversation as a tree of exchanges, and the path the panel shows.
//
// A follow-up is a child of the exchange it follows. Regenerating or editing a
// question adds a sibling — another version — and the panel shows one version
// per parent, navigable ‹ 2/3 ›. What follows a version belongs to it: going
// back to version 1 shows what followed version 1.

import { emptyExchange, exchangeOf, reduce } from "./transcript"
import type { Exchange } from "./transcript"
import type { AiUpdate, ThreadView } from "@/lib/ipc/ai"

export interface ExchangeNode {
  id: number
  parent: number | null
  exchange: Exchange
}

/** A message typed while a run was going: sent when it ends, never before. */
export interface QueuedMessage {
  key: string
  text: string
}

export interface Thread {
  /** `null` until the backend created it, on the first question. */
  id: string | null
  title: string
  nodes: Array<ExchangeNode>
  /** The version shown under each parent; `root` for the first exchange. */
  selections: Record<string, number>
  running: number | null
  queue: Array<QueuedMessage>
}

export const NEW_THREAD: Thread = {
  id: null,
  title: "",
  nodes: [],
  selections: {},
  running: null,
  queue: [],
}

export function selectionKey(parent: number | null) {
  return parent === null ? "root" : String(parent)
}

export function threadOf(view: ThreadView): Thread {
  return {
    id: view.id,
    title: view.title,
    nodes: view.nodes.map((node) => ({
      id: node.id,
      parent: node.parent,
      exchange: {
        ...exchangeOf(node.question, node.events),
        // A run the backend no longer drives is not running, whatever its log
        // says: only `view.running` knows.
        running: view.running === node.id,
      },
    })),
    selections: Object.fromEntries(
      view.selections.map((selection) => [
        selectionKey(selection.parent),
        selection.node,
      ])
    ),
    running: view.running,
    queue: [],
  }
}

/** Opens a node the backend just accepted, and shows it. */
export function addNode(
  thread: Thread,
  id: number,
  parent: number | null,
  question: string
): Thread {
  if (thread.nodes.some((node) => node.id === id)) return thread
  const exchange = { ...emptyExchange(question), running: true }
  return {
    ...thread,
    title: thread.title === "" && parent === null ? question : thread.title,
    nodes: [...thread.nodes, { id, parent, exchange }],
    selections: { ...thread.selections, [selectionKey(parent)]: id },
    running: id,
  }
}

/** Folds a streamed event into its node. An unknown node is left to the caller. */
export function applyUpdate(thread: Thread, update: AiUpdate): Thread {
  if (!thread.nodes.some((node) => node.id === update.node)) return thread
  const nodes = thread.nodes.map((node) =>
    node.id === update.node
      ? { ...node, exchange: reduce(node.exchange, update.event) }
      : node
  )
  const ended =
    update.event.kind === "finished" || update.event.kind === "failed"
  return {
    ...thread,
    nodes,
    running: ended && thread.running === update.node ? null : thread.running,
  }
}

export function updateExchange(
  thread: Thread,
  node: number,
  change: (exchange: Exchange) => Exchange
): Thread {
  return {
    ...thread,
    nodes: thread.nodes.map((found) =>
      found.id === node ? { ...found, exchange: change(found.exchange) } : found
    ),
  }
}

function childrenOf(thread: Thread, parent: number | null) {
  return thread.nodes
    .filter((node) => node.parent === parent)
    .sort((a, b) => a.id - b.id)
}

/** The exchanges shown, first to last. */
export function activePath(thread: Thread): Array<ExchangeNode> {
  const path: Array<ExchangeNode> = []
  let parent: number | null = null
  // Bounded by the node count: a malformed selection cannot loop.
  for (let depth = 0; depth <= thread.nodes.length; depth += 1) {
    const children = childrenOf(thread, parent)
    if (children.length === 0) break
    const chosen: number | undefined = thread.selections[selectionKey(parent)]
    const node: ExchangeNode | undefined =
      children.find((child) => child.id === chosen) ?? children.at(-1)
    if (!node) break
    path.push(node)
    parent = node.id
  }
  return path
}

export interface Versions {
  /** 1-based position among its siblings. */
  position: number
  count: number
  previous: number | null
  next: number | null
}

export function versionsOf(thread: Thread, node: ExchangeNode): Versions {
  const siblings = childrenOf(thread, node.parent)
  const index = siblings.findIndex((sibling) => sibling.id === node.id)
  return {
    position: index + 1,
    count: siblings.length,
    previous: siblings[index - 1]?.id ?? null,
    next: siblings[index + 1]?.id ?? null,
  }
}

export function select(thread: Thread, node: number): Thread {
  const found = thread.nodes.find((candidate) => candidate.id === node)
  if (!found) return thread
  return {
    ...thread,
    selections: { ...thread.selections, [selectionKey(found.parent)]: node },
  }
}

export function enqueue(thread: Thread, key: string, text: string): Thread {
  return { ...thread, queue: [...thread.queue, { key, text }] }
}

export function dequeue(thread: Thread, key: string): Thread {
  return {
    ...thread,
    queue: thread.queue.filter((message) => message.key !== key),
  }
}

/** The exchange shown last, if any. */
export function lastExchange(thread: Thread) {
  return activePath(thread).at(-1) ?? null
}
