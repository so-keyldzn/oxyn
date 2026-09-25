import * as React from "react"
import { useStore } from "@tanstack/react-store"

import type { ExecutionSummary } from "@/components/oxyn/status-bar"
import {
  consoleTitle,
  cycleConsole,
  needsCloseDecision,
} from "@/features/consoles/console-model"
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
type SeedInput = Partial<ConsoleSeed> & { notice?: string | null }

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
    const document = input.document ?? (await library.newDocument())
    const entry: ConsoleEntry = {
      key,
      session,
      origin,
      notice: input.notice ?? null,
      seed: {
        document,
        revision: input.revision ?? 0,
        title: input.title ?? consoleTitle(counter.current),
        text: input.text ?? "",
        savedTitle: input.savedTitle ?? null,
        savedText: input.savedText ?? null,
        hasSavedCopy: input.hasSavedCopy ?? false,
        fromAgent: input.fromAgent ?? false,
        parameters: input.parameters ?? [],
        needsValues: input.needsValues ?? false,
      },
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
    const closed = await handle.close(discard)
    if (!closed) return false
    // The session goes with the console; siblings and the catalog keep theirs.
    if (entry.session)
      void consoles
        .close(open.connection, entry.session.session)
        .catch(() => undefined)
    const rest = entriesRef.current.filter((item) => item.key !== key)
    setEntries(rest)
    setMeta(({ [key]: _gone, ...others }) => others)
    const last = rest[rest.length - 1]
    if (last) onActivate(last.key)
    return true
  }

  const requestClose = (key: string) => {
    const handle = handles.current.get(key)
    if (!handle) return
    const reasons = handle.closeReasons()
    const activity = meta[key]?.activity
    if (activity === "saving" || activity === "closing") {
      setNotice("Wait for the current save or close operation, or cancel it.")
      return
    }
    if (needsCloseDecision(reasons)) setClosing({ key, reasons, busy: false })
    else void finishClose(key, false)
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

  // The first console uses the session opened with the connection, and the
  // SQL carried from the previous connection — never run.
  const started = React.useRef(false)
  const [ready, setReady] = React.useState(false)
  React.useEffect(() => {
    if (started.current) return
    started.current = true
    void (async () => {
      await add(open.console, { text: sessionStore.state.sqlDraft })
      setReady(true)
    })()
    // Once per workspace: the screen is keyed by the connection's session.
  }, [])

  // Working copies chosen on the recovery screen, whenever they are chosen —
  // at startup, or from this workspace and back. Each opens offline: text
  // only, no session, nothing run. A workspace on its way out leaves them to
  // the next one.
  const restored = useStore(sessionStore, (state) => state.restored.length)
  React.useEffect(() => {
    if (!ready || restored === 0) return
    if (sessionStore.state.open?.session !== open.session) return
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
  }, [ready, restored])

  // Text from the assistant or the inspector, dropped into a console unrun.
  const requests = useStore(consoleTextRequests)
  const activeRef = React.useRef<string | null>(null)
  React.useEffect(() => {
    if (requests.length === 0) return
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
  }, [requests])

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
    cycle,
    openHistoryCopy,
    openDocumentCopy,
    resume,
  }
}
