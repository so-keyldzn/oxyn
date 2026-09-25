import * as React from "react"
import { useStore } from "@tanstack/react-store"

import type { ExecutionSummary } from "@/components/oxyn/status-bar"
import {
  consoleTitle,
  cycleConsole,
  needsCloseDecision,
  tabTitle,
} from "@/features/consoles/console-model"
import { publishConsoleLabels } from "@/features/consoles/console-labels"
import type { CloseReasons } from "@/features/consoles/console-model"
import type {
  ConsoleHandle,
  ConsoleMeta,
} from "@/features/consoles/console-panel"
import {
  consoleTextRequests,
  takeConsoleTextRequests,
} from "@/features/consoles/open-in-console"
import type { ConsoleSeed } from "@/features/consoles/use-console-document"
import {
  session as sessionStore,
  takeRestoredWorkingCopies,
} from "@/features/session"
import { BackendError, backend, newCommandId } from "@/lib/ipc/client"
import { consoles } from "@/lib/ipc/consoles"
import type { ConsoleSession } from "@/lib/ipc/consoles"
import { forgetSession, recordTransactionState } from "@/lib/ipc/events"
import { library } from "@/lib/ipc/library"
import type { DocumentView, HistoryDetail } from "@/lib/ipc/library"
import type { OpenConnection } from "@/lib/ipc/types"

export interface ConsoleEntry {
  key: string
  /**
   * `null` while the console is offline: a restored working copy edits,
   * saves and closes, but runs nothing until the user attaches it.
   */
  session: ConsoleSession | null
  /**
   * The connection the document is saved under. An offline copy keeps the
   * one it was written for, so that attaching it there later resumes the
   * same document rather than a copy.
   */
  origin: string | null
  seed: ConsoleSeed
  notice: string | null
}

/** What a new console starts with; a new document unless one is resumed. */
export type SeedInput = Partial<ConsoleSeed> & { notice?: string | null }

/** The seed of a new console: what `input` says, a new empty console else. */
/** A console that would lose work if its window closed. */
export interface WindowCloseCost {
  title: string
  unsaved: boolean
  /** The stored document changed elsewhere and would be left untouched. */
  conflict: boolean
  running: boolean
}

export function consoleSeed(
  input: SeedInput,
  document: string,
  title: string
): ConsoleSeed {
  return {
    document,
    revision: input.revision ?? 0,
    title: input.title ?? title,
    text: input.text ?? "",
    savedTitle: input.savedTitle ?? null,
    savedText: input.savedText ?? null,
    hasSavedCopy: input.hasSavedCopy ?? false,
    fromAgent: input.fromAgent ?? false,
    parameters: input.parameters ?? [],
    needsValues: input.needsValues ?? false,
    context: input.context ?? null,
  }
}

/** A closed console as ⌘⇧T brings it back: its text, never its run. */
interface ClosedConsole {
  title: string
  text: string
  /** Kept: a reopened agent's text stays marked as one (ADR-0023). */
  fromAgent: boolean
}

/** Closed consoles kept for ⌘⇧T, the oldest forgotten first. */
const MAX_REOPENABLE = 20

function failure(error: unknown) {
  return error instanceof BackendError ? error.message : String(error)
}

function resumeSeed(document: DocumentView): SeedInput {
  return {
    document: document.id,
    revision: Math.max(document.revision, document.savedRevision),
    title: document.title,
    text: document.text,
    savedTitle: document.savedTitle,
    savedText: document.savedText,
    hasSavedCopy: document.isSaved,
    fromAgent: document.fromAgent,
  }
}

/**
 * The consoles of one open connection: each with its own session and
 * document (ADR-0015). Opening is cancellable and runs no SQL; closing asks
 * when work would be lost and closes the console's session.
 */
export function useConsoles({
  open,
  onActivate,
}: {
  open: OpenConnection
  /** A console became the one shown, or needs to be. */
  onActivate: (key: string) => void
}) {
  const [entries, setEntries] = React.useState<Array<ConsoleEntry>>([])
  const [meta, setMeta] = React.useState<Record<string, ConsoleMeta>>({})
  const [opening, setOpening] = React.useState<string | null>(null)
  /** The offline console a session is being opened for. */
  const [attaching, setAttaching] = React.useState<string | null>(null)
  const [notice, setNotice] = React.useState<string | null>(null)
  const [closing, setClosing] = React.useState<{
    key: string
    reasons: CloseReasons
    busy: boolean
  } | null>(null)
  const [summary, setSummary] = React.useState<ExecutionSummary>({
    status: "idle",
  })
  const handles = React.useRef(new Map<string, ConsoleHandle>())
  const counter = React.useRef(0)
  const openingRef = React.useRef<string | null>(null)
  const entriesRef = React.useRef(entries)
  entriesRef.current = entries
  const metaRef = React.useRef(meta)
  metaRef.current = meta
  const [closedConsoles, setClosedConsoles] = React.useState<
    Array<ClosedConsole>
  >([])

  const register = React.useCallback(
    (key: string, handle: ConsoleHandle | null) => {
      if (handle) handles.current.set(key, handle)
      else handles.current.delete(key)
    },
    []
  )
  const onMeta = React.useCallback((key: string, next: ConsoleMeta) => {
    setMeta((all) => ({ ...all, [key]: next }))
  }, [])

  const add = async (
    session: ConsoleSession | null,
    input: SeedInput,
    origin: string | null = open.connection
  ) => {
    counter.current += 1
    const key = `console:${counter.current}`
    // What the session reported at opening; events carry it on (ADR-0039).
    if (session)
      recordTransactionState(session.session, session.transactionState)
    const document = input.document ?? (await library.newDocument())
    const entry: ConsoleEntry = {
      key,
      session,
      origin,
      notice: input.notice ?? null,
      seed: consoleSeed(input, document, consoleTitle(counter.current)),
    }
    setEntries((all) => [...all, entry])
    onActivate(key)
    return key
  }

  /**
   * Opens a console session, then hands it to `use`. One opening at a time;
   * a cancelled opening closes the session the server still returned.
   */
  const withSession = async <T>(
    use: (session: ConsoleSession) => Promise<T> | T
  ) => {
    if (openingRef.current) {
      setNotice(
        "Finish or cancel the current console opening before opening another query."
      )
      return null
    }
    const id = newCommandId()
    openingRef.current = id
    setOpening(id)
    setNotice("Opening an independent console session…")
    try {
      const session = await consoles.open(id, open.connection)
      if (openingRef.current !== id) {
        // Cancelled or replaced while the server answered: nobody uses it.
        void consoles
          .close(open.connection, session.session)
          .catch(() => undefined)
        return null
      }
      setNotice(null)
      return await use(session)
    } catch (error) {
      if (openingRef.current === id) setNotice(failure(error))
      return null
    } finally {
      if (openingRef.current === id) {
        openingRef.current = null
        setOpening(null)
      }
    }
  }

  /** Opens a console with its own session. One opening at a time. */
  const openConsole = (input: SeedInput = {}) =>
    withSession((session) => add(session, input))

  /**
   * Attaches an offline console to this connection: the separate gesture
   * UX-SPEC asks for. On the connection its document was written for, the
   * **same** console gets a session — its editor, undo history and write
   * queue stay. On another one, a copy opens and the original stays offline,
   * its document untouched. Nothing runs either way.
   */
  const attach = async (key: string) => {
    const entry = entriesRef.current.find((item) => item.key === key)
    if (!entry || entry.session !== null || openingRef.current) {
      if (openingRef.current)
        setNotice(
          "Finish or cancel the current console opening before connecting this query."
        )
      return null
    }
    setAttaching(key)
    try {
      if (entry.origin !== open.connection) {
        const handle = handles.current.get(key)
        return await openConsole({
          title: meta[key]?.title ?? entry.seed.title,
          text: handle?.text() ?? entry.seed.text,
          fromAgent: entry.seed.fromAgent,
          notice:
            "Connected a new copy. The original remains saved locally. Nothing was executed.",
        })
      }
      return await withSession((session) => {
        recordTransactionState(session.session, session.transactionState)
        setEntries((all) =>
          all.map((item) => (item.key === key ? { ...item, session } : item))
        )
        setNotice("Recovered query connected. Nothing was executed.")
        onActivate(key)
        return key
      })
    } finally {
      setAttaching((current) => (current === key ? null : current))
    }
  }

  const cancelOpening = () => {
    const id = openingRef.current
    if (!id) return
    openingRef.current = null
    setOpening(null)
    void backend.cancel(id)
    setNotice("Opening the console was cancelled.")
  }

  const finishClose = async (key: string, discard: boolean) => {
    const handle = handles.current.get(key)
    const entry = entriesRef.current.find((item) => item.key === key)
    if (!handle || !entry) return false
    // Read before closing: the text is what ⌘⇧T brings back, discarded
    // text included — the reopening is how a hasty Discard is undone.
    const shown = metaRef.current[key]
    const reopenable: ClosedConsole = {
      title: shown?.title ?? entry.seed.title,
      text: handle.text(),
      fromAgent: shown?.fromAgent ?? entry.seed.fromAgent,
    }
    const closed = await handle.close(discard)
    if (!closed) return false
    setClosedConsoles((all) => [...all, reopenable].slice(-MAX_REOPENABLE))
    // The session goes with the console; siblings and the catalog keep theirs.
    if (entry.session) {
      void consoles
        .close(open.connection, entry.session.session)
        .catch(() => undefined)
      forgetSession(entry.session.session)
    }
    const rest = entriesRef.current.filter((item) => item.key !== key)
    // Now rather than at the next render: a series of closes reads it.
    entriesRef.current = rest
    setEntries(rest)
    setMeta(({ [key]: _gone, ...others }) => others)
    const last = rest[rest.length - 1]
    if (last) onActivate(last.key)
    return true
  }

  // Resolves the close the dialog is deciding: `true` once the console is
  // gone, `false` on Cancel — which ends a series of closes there.
  const settle = React.useRef<((closed: boolean) => void) | null>(null)
  const settleClose = (closed: boolean) => {
    const resolve = settle.current
    settle.current = null
    resolve?.(closed)
  }

  /**
   * Closes the console `key`, asking first when work would be lost
   * (ADR-0015). Resolves whether it closed: a series of closes stops at the
   * first that did not (UX-SPEC, « Menus contextuels », Onglet).
   */
  const requestClose = (key: string): Promise<boolean> => {
    const handle = handles.current.get(key)
    if (!handle) return Promise.resolve(false)
    const reasons = handle.closeReasons()
    const activity = metaRef.current[key]?.activity
    if (activity === "saving" || activity === "closing") {
      setNotice("Wait for the current save or close operation, or cancel it.")
      return Promise.resolve(false)
    }
    if (!needsCloseDecision(reasons)) return finishClose(key, false)
    settleClose(false)
    // The console the dialog names is the one shown behind it.
    onActivate(key)
    setClosing({ key, reasons, busy: false })
    return new Promise((resolve) => {
      settle.current = resolve
    })
  }

  /**
   * What closing the window would cost, console by console: those that would
   * lose text or stop a statement (ADR-0043). A transaction is not listed
   * here: the backend has had it resolved before the window asks.
   */
  const windowCloseCosts = (): Array<WindowCloseCost> =>
    entriesRef.current.flatMap((entry) => {
      const handle = handles.current.get(entry.key)
      if (!handle) return []
      const reasons = handle.closeReasons()
      const activity = metaRef.current[entry.key]?.activity
      const running =
        reasons.running || activity === "saving" || activity === "closing"
      if (!reasons.unsaved && !reasons.conflict && !running) return []
      return [
        {
          title: reasons.title,
          unsaved: reasons.unsaved,
          conflict: reasons.conflict,
          running,
        },
      ]
    })

  /**
   * Closes every console of the workspace once its window's close is
   * confirmed: the window's dialog was the decision, so none asks again.
   * Unsaved text is discarded; a running statement is cancelled as `⌘W`
   * would, and a write whose outcome turns unknown stays flagged, never
   * replayed (I-13).
   */
  const closeAll = async () => {
    for (const entry of [...entriesRef.current]) {
      await finishClose(entry.key, true).catch(() => false)
    }
  }

  // Bumped by every decision: an answer to a decision since cancelled neither
  // closes the console nor reopens the dialog (UX-SPEC « Sauvegarde d'une
  // console »).
  const decision = React.useRef(0)
  const decideClose = async (choice: "cancel" | "save" | "discard") => {
    const current = closing
    if (!current) return
    decision.current += 1
    if (choice === "cancel") {
      setClosing(null)
      if (current.busy) handles.current.get(current.key)?.cancelWrite()
      settleClose(false)
      return
    }
    const token = decision.current
    setClosing({ ...current, busy: true })
    const handle = handles.current.get(current.key)
    const saved = choice === "discard" || (handle ? await handle.save() : false)
    if (decision.current !== token) return
    // A close the store committed despite a late cancellation still removes
    // the tab: its document is gone, only the dialog stays shut.
    const closed =
      saved && (await finishClose(current.key, choice === "discard"))
    if (decision.current !== token) return
    setClosing(closed ? null : { ...current, busy: false })
    if (closed) settleClose(true)
  }

  /**
   * ⌘⇧T: the last closed console again, in a console of its own — its own
   * session, its text, nothing run. Taken off the list only once opened: a
   * refused or cancelled opening leaves it there.
   */
  const reopen = async () => {
    const last = closedConsoles[closedConsoles.length - 1]
    if (!last) return null
    const key = await openConsole({
      title: last.title,
      text: last.text,
      fromAgent: last.fromAgent,
      notice: "Closed console reopened. Nothing was executed.",
    })
    if (key)
      setClosedConsoles((all) =>
        all[all.length - 1] === last ? all.slice(0, -1) : all
      )
    return key
  }

  /**
   * `Duplicate`: an independent console — its own session and document —
   * with the same text, nothing run (UX-SPEC, « Menus contextuels »).
   */
  const duplicate = (key: string) => {
    const entry = entriesRef.current.find((item) => item.key === key)
    const handle = handles.current.get(key)
    if (!entry || !handle) return Promise.resolve(null)
    const shown = metaRef.current[key]
    return openConsole({
      text: handle.text(),
      fromAgent: shown?.fromAgent ?? entry.seed.fromAgent,
      notice: `Duplicated from ${shown?.title ?? entry.seed.title}. Nothing was executed.`,
    })
  }

  const cycle = (active: string | null, delta: 1 | -1) =>
    cycleConsole(
      entriesRef.current.map((entry) => entry.key),
      active,
      delta
    )

  const openHistoryCopy = async (entry: HistoryDetail) => {
    // Refused here, whatever button asked: an editable copy of a write whose
    // outcome is unknown is one ⌘↵ away from applying it twice (I-13).
    if (entry.needsInspection) {
      setNotice(
        "This write needs inspection: it stays readable in the library and is never opened as an editable copy."
      )
      return null
    }
    return openConsole({
      title: "History copy.sql",
      text: entry.statement,
      fromAgent: entry.fromAgent,
      // Names both ends: where the text ran, and where it would run now.
      notice: `Copy from ${entry.fromAgent ? "an agent's History entry" : "History"} · ${entry.connectionName ?? "Connection unavailable"}. Review for ${open.name} before running; nothing was executed.`,
    })
  }

  const openDocumentCopy = (document: DocumentView, origin: string) =>
    openConsole({
      title: document.savedTitle ?? (document.title || "Query copy.sql"),
      text: document.savedText ?? document.text,
      fromAgent: document.fromAgent,
      notice: `Copy from Saved query · ${origin}. Review for ${open.name} before running; nothing was executed.`,
    })

  const resume = (document: DocumentView) => {
    const holder = entriesRef.current.find(
      (entry) =>
        handles.current.has(entry.key) && entry.seed.document === document.id
    )
    if (holder) {
      onActivate(holder.key)
      setNotice(
        "The existing console was preserved, including its unsaved changes."
      )
      return Promise.resolve(holder.key)
    }
    return openConsole({
      ...resumeSeed(document),
      notice: "Working copy resumed. Nothing was executed.",
    })
  }

  // The first console uses the session opened with the connection, and a
  // copy of the SQL the previous workspace showed — announced, never run.
  // That workspace keeps its own console (UX-SPEC).
  const started = React.useRef(false)
  const [ready, setReady] = React.useState(false)
  React.useEffect(() => {
    if (started.current) return
    started.current = true
    void (async () => {
      const { sqlDraft, sqlDraftFrom } = sessionStore.state
      await add(open.console, {
        text: sqlDraft,
        notice:
          sqlDraftFrom === null
            ? null
            : `Copied from ${sqlDraftFrom}, which keeps its console. Nothing was executed.`,
      })
      setReady(true)
    })()
    // Once per workspace: the screen is keyed by the connection's session.
  }, [])

  // Working copies chosen on the recovery screen, whenever they are chosen —
  // at startup, or from this workspace and back. Each opens offline: text
  // only, no session, nothing run. A workspace on its way out leaves them to
  // the next one.
  const restored = useStore(sessionStore, (state) => state.restored.length)
  // What reaches « the workspace » — recovered copies, text for the active
  // console — goes to the shown one only (ADR-0046).
  const shown = useStore(
    sessionStore,
    (state) => state.open?.session === open.session
  )
  React.useEffect(() => {
    if (!ready || restored === 0) return
    if (!shown) return
    void (async () => {
      for (const entry of takeRestoredWorkingCopies()) {
        const holder = entriesRef.current.find(
          (item) => item.seed.document === entry.id
        )
        if (holder) {
          onActivate(holder.key)
          continue
        }
        try {
          const document = await library.openDocument(entry.id)
          await add(
            null,
            {
              ...resumeSeed(document),
              notice:
                "Recovered offline. Choose a connection before running. Nothing was executed.",
            },
            document.connection
          )
        } catch (error) {
          setNotice(failure(error))
        }
      }
    })()
  }, [ready, restored, shown])

  // Text from the assistant or the inspector, dropped into a console unrun.
  const requests = useStore(consoleTextRequests)
  const activeRef = React.useRef<string | null>(null)
  React.useEffect(() => {
    if (requests.length === 0) return
    // Every retained workspace listens; only the shown one takes the text,
    // or it would land unseen in a hidden console (ADR-0046).
    if (!shown) return
    for (const request of takeConsoleTextRequests()) {
      const target = activeRef.current
        ? handles.current.get(activeRef.current)
        : undefined
      if (request.newConsole || !target) {
        void openConsole({ text: request.sql, notice: request.notice })
      } else if (activeRef.current) {
        onActivate(activeRef.current)
        target.insert(request.sql, request.notice)
      }
    }
  }, [requests, shown])

  // The names the exit dialog shows for these consoles' sessions.
  React.useEffect(() => {
    const labels: Record<string, string> = {}
    for (const entry of entries) {
      if (!entry.session) continue
      const drawn = meta[entry.key]
      labels[entry.session.session] = tabTitle({
        title: drawn?.title ?? entry.seed.title,
        fromAgent: drawn?.fromAgent ?? entry.seed.fromAgent,
      })
    }
    publishConsoleLabels([], labels)
    return () => publishConsoleLabels(Object.keys(labels), {})
  }, [entries, meta])

  return {
    entries,
    meta,
    opening: opening !== null,
    attaching,
    notice,
    closing,
    summary,
    setSummary,
    register,
    onMeta,
    handles,
    /** The console text requests go to; set by the screen on each render. */
    activeRef,
    openConsole,
    attach,
    cancelOpening,
    requestClose,
    decideClose,
    windowCloseCosts,
    closeAll,
    /** A closed console can be brought back by ⌘⇧T. */
    reopenable: closedConsoles.length > 0,
    reopen,
    duplicate,
    cycle,
    openHistoryCopy,
    openDocumentCopy,
    resume,
  }
}
