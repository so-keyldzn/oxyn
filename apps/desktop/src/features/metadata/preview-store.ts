import { createStore } from "@tanstack/react-store"

import type { ResultState } from "@/components/oxyn/result-panel"
import { stateFromOutcome } from "@/features/workspace/use-execution"
import { PLAIN_SHAPE } from "@/lib/ipc/metadata"
import type { PreviewShape } from "@/lib/ipc/metadata"
import type { CatalogAddress, CommandOutcome } from "@/lib/ipc/types"

/** Previews kept once read: each holds at most 200 rows, and a result. */
export const KEPT_PREVIEWS = 8

/** One relation's preview, alive across the object view's mounts. */
export interface PreviewEntry {
  connection: string
  session: string
  address: CatalogAddress
  /** Whether a cancel reaches the server, or only stops reading here. */
  serverCancel: boolean
  /** The command reading now, or `null`. */
  running: string | null
  cancelling: boolean
  startedAt: number | null
  state: ResultState
  /** The shape the rows on screen came from. */
  applied: PreviewShape
  /** The shape of the read in flight. */
  requested: PreviewShape
  /** The policy asked for an approval, and the preview refused it. */
  approvalRefused: boolean
  /** Something changed on the connection since this was read. */
  stale: boolean
  /** Views showing it now; an entry nobody shows may be evicted. */
  views: number
  /** For eviction: the entry least recently shown goes first. */
  shownAt: number
}

/** What the store asks of the backend; injected, so it is tested without IPC. */
export interface PreviewEffects {
  read: (
    id: string,
    entry: PreviewEntry,
    shape: PreviewShape
  ) => Promise<CommandOutcome>
  cancel: (id: string) => void
  refuse: (command: string) => void
  forgetResult: (result: string) => void
  watch: (id: string) => void
  unwatch: (id: string) => void
  newId: () => string
  now: () => number
}

export type Previews = Record<string, PreviewEntry>

export function previewKey(connection: string, relationKey: string) {
  return `${connection}\u0000${relationKey}`
}

function resultOf(entry: PreviewEntry | undefined) {
  return entry?.state.status === "populated" ? entry.state.result : null
}

/**
 * The preview store: which relation previews exist, and what each shows.
 *
 * A preview is **kept** once read, so that coming back to an object does not
 * read it again (docs/UX-SPEC.md, « Données d'une table sélectionnée »), and
 * **cancelled** when the last view showing it goes away mid-read: work nobody
 * will see is not left running at the server. A late answer — from a read
 * that was replaced, cancelled or abandoned — is never shown, and its result
 * is released. Previews beyond [`KEPT_PREVIEWS`] release theirs, oldest
 * unseen first.
 */
export function createPreviewStore(effects: PreviewEffects) {
  const store = createStore<Previews>({})

  const update = (key: string, change: (entry: PreviewEntry) => PreviewEntry) =>
    store.setState((all) => {
      const entry = all[key]
      return entry ? { ...all, [key]: change(entry) } : all
    })

  const evict = () => {
    const entries = Object.entries(store.state)
    const unseen = entries
      .filter(([, entry]) => entry.views === 0 && entry.running === null)
      .sort(([, a], [, b]) => a.shownAt - b.shownAt)
    let excess = entries.length - KEPT_PREVIEWS
    const gone: Array<string> = []
    for (const [key, entry] of unseen) {
      if (excess <= 0) break
      const result = resultOf(entry)
      if (result) effects.forgetResult(result)
      gone.push(key)
      excess--
    }
    if (gone.length === 0) return
    store.setState((all) => {
      const next = { ...all }
      for (const key of gone) delete next[key]
      return next
    })
  }

  const settle = (key: string, id: string, outcome: CommandOutcome) => {
    effects.unwatch(id)
    const entry = store.state[key]
    if (entry?.running !== id) {
      // Replaced, cancelled or abandoned: never shown, and nothing held.
      if (outcome.type === "executed") effects.forgetResult(outcome.result)
      if (outcome.type === "needsApproval") effects.refuse(outcome.command)
      return
    }
    if (outcome.type === "needsApproval") {
      // A preview never waits for an approval: the console is where to run it.
      effects.refuse(outcome.command)
      update(key, (current) => ({
        ...current,
        running: null,
        cancelling: false,
        startedAt: null,
        state: { status: "initial" },
        approvalRefused: true,
      }))
      return
    }
    const { state } = stateFromOutcome(outcome)
    update(key, (current) => ({
      ...current,
      running: null,
      cancelling: false,
      startedAt: null,
      state:
        outcome.type === "executed" && state.status === "populated"
          ? { ...state, elapsedMs: outcome.elapsedMs }
          : state,
      // Only rows commit a shape: a refused read leaves the one in force.
      applied:
        outcome.type === "executed" ? current.requested : current.applied,
    }))
    evict()
  }

  const fail = (
    key: string,
    id: string,
    message: string,
    retryable: boolean
  ) => {
    effects.unwatch(id)
    if (store.state[key]?.running !== id) return
    update(key, (current) => ({
      ...current,
      running: null,
      cancelling: false,
      startedAt: null,
      state: { status: "error", message, retryable },
    }))
  }

  return {
    store,

    /** Creates the entry if needed and marks one more view showing it. */
    attach(
      key: string,
      target: {
        connection: string
        session: string
        address: CatalogAddress
        serverCancel: boolean
      }
    ) {
      store.setState((all) => {
        const entry = all[key]
        return {
          ...all,
          [key]: entry
            ? { ...entry, views: entry.views + 1, shownAt: effects.now() }
            : {
                ...target,
                running: null,
                cancelling: false,
                startedAt: null,
                state: { status: "initial" },
                applied: PLAIN_SHAPE,
                requested: PLAIN_SHAPE,
                approvalRefused: false,
                stale: false,
                views: 1,
                shownAt: effects.now(),
              },
        }
      })
    },

    /**
     * One view fewer. The last one leaving cancels a read in flight — its
     * answer will be released on arrival — and drops an entry that never
     * held rows.
     */
    detach(key: string) {
      const entry = store.state[key]
      if (!entry) return
      const views = Math.max(0, entry.views - 1)
      if (views > 0) {
        update(key, (current) => ({ ...current, views }))
        return
      }
      if (entry.running) effects.cancel(entry.running)
      if (entry.running || resultOf(entry) === null) {
        store.setState((all) => {
          const { [key]: _gone, ...rest } = all
          return rest
        })
        return
      }
      update(key, (current) => ({ ...current, views }))
      evict()
    },

    /** Reads `shape`, replacing — and cancelling — any read in flight. */
    read(key: string, shape: PreviewShape) {
      const entry = store.state[key]
      if (!entry) return
      if (entry.running) effects.cancel(entry.running)
      const previous = resultOf(entry)
      if (previous) effects.forgetResult(previous)
      const id = effects.newId()
      effects.watch(id)
      update(key, (current) => ({
        ...current,
        running: id,
        cancelling: false,
        startedAt: effects.now(),
        requested: shape,
        approvalRefused: false,
        // A change signalled during this read keeps it stale: owed once more.
        stale: false,
        state: {
          status: "running",
          rows: 0,
          serverCancel: current.serverCancel,
        },
      }))
      effects.read(id, entry, shape).then(
        (outcome) => settle(key, id, outcome),
        (error: unknown) =>
          fail(
            key,
            id,
            error instanceof Error ? error.message : String(error),
            typeof error === "object" &&
              error !== null &&
              "retryable" in error &&
              error.retryable === true
          )
      )
    },

    cancel(key: string) {
      const entry = store.state[key]
      if (!entry?.running || entry.cancelling) return
      effects.cancel(entry.running)
      update(key, (current) => ({ ...current, cancelling: true }))
    },

    /** Something changed on `connection`: every preview of it is stale. */
    invalidate(connection: string) {
      store.setState((all) => {
        let changed = false
        const next = { ...all }
        for (const [key, entry] of Object.entries(all)) {
          if (entry.connection !== connection && connection !== "*") continue
          changed = true
          next[key] = { ...entry, stale: true }
        }
        return changed ? next : all
      })
    },
  }
}

export type PreviewStore = ReturnType<typeof createPreviewStore>
