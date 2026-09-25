import * as React from "react"
import { useDebouncer } from "@tanstack/react-pacer"
import { useQueryClient } from "@tanstack/react-query"

import type { SaveState } from "@/components/oxyn/console-toolbar"
import { saveNotice, titleTooLong } from "@/features/consoles/console-model"
import { refreshLibrary } from "@/features/library/library-refresh"
import { backend, newCommandId } from "@/lib/ipc/client"
import type { ParameterInput } from "@/lib/ipc/consoles"
import { library } from "@/lib/ipc/library"
import type { DocumentWrite } from "@/lib/ipc/library"

/**
 * The typing pause after which the draft is written: under the 300 ms of
 * visible feedback, above the interval of fast typing, so a sentence typed in
 * one go writes once (ADR-0024).
 */
export const DRAFT_IDLE_MS = 250

const TITLE_TOO_LONG = "Query names must not exceed 256 UTF-8 bytes."

/** Where a console's text comes from when it opens. */
export interface ConsoleSeed {
  document: string
  /** The last revision the store acknowledged for this document, 0 if new. */
  revision: number
  title: string
  text: string
  savedTitle: string | null
  savedText: string | null
  hasSavedCopy: boolean
  fromAgent: boolean
  /**
   * Values to bind, pre-filled in the Parameters panel. They come from the
   * user's own data: bound by the driver, never written into the text, never
   * saved with the query and never logged (I-03, I-10).
   */
  parameters: Array<ParameterInput>
  /** A value is still missing: the console says so before anything runs. */
  needsValues: boolean
}

interface Draft {
  title: string
  text: string
}

/** A named save or close; `cancelled` is set by `cancelWrite`, meanwhile. */
interface Cancellable {
  id: string
  cancelled: boolean
}

function cancellable(): Cancellable {
  return { id: newCommandId(), cancelled: false }
}

/**
 * Read after each await: the type checker keeps a property it saw false as
 * false across an await, while `cancelWrite` may have set it meanwhile.
 */
function isCancelled(request: Cancellable) {
  return request.cancelled
}

function message(error: unknown) {
  return error instanceof Error ? error.message : String(error)
}

/**
 * One console's query document: working draft, named copy, revision check.
 *
 * The backend owns the write queue; this hook only numbers revisions and says
 * what happened. A conflict freezes writes and keeps the local text: the only
 * way forward is a new copy (ADR-0016).
 */
export function useConsoleDocument({
  seed,
  connection,
}: {
  seed: ConsoleSeed
  /** The connection the document is saved under; `null` for none. */
  connection: string | null
}) {
  const queryClient = useQueryClient()
  const [document, setDocument] = React.useState(seed.document)
  const [title, setTitle] = React.useState(seed.title)
  const [text, setText] = React.useState(seed.text)
  const [savedTitle, setSavedTitle] = React.useState(seed.savedTitle ?? "")
  const [savedText, setSavedText] = React.useState(seed.savedText ?? "")
  const [hasSavedCopy, setHasSavedCopy] = React.useState(seed.hasSavedCopy)
  const [conflict, setConflict] = React.useState(false)
  const [saving, setSaving] = React.useState(false)
  const [closing, setClosing] = React.useState(false)
  const [notice, setNotice] = React.useState<string | null>(null)
  /** The last save or close did not go through: its notice is a warning. */
  const [problem, setProblem] = React.useState(false)
  const [draftNotice, setDraftNotice] = React.useState(
    "Draft recovery has not been written yet."
  )

  // Read by writes that outlive the render which started them.
  const latest = React.useRef({ document, title, text, conflict, closing })
  latest.current = { document, title, text, conflict, closing }
  const revision = React.useRef(seed.revision)
  /**
   * The draft the store holds or is writing. `null` for a new document whose
   * seed was never written: a copy opened from History or a saved query is
   * recoverable before its first edit (UX-SPEC « Autosauvegarde des
   * brouillons »).
   */
  const lastDraft = React.useRef<Draft | null>(
    seed.revision === 0 && seed.text !== ""
      ? null
      : { title: seed.title, text: seed.text }
  )
  /** The draft write under way: a flush finding its text sent awaits it. */
  const pendingDraft = React.useRef<{
    sent: Draft
    done: Promise<boolean>
  } | null>(null)
  /** Numbers draft attempts, refused ones included. */
  const draftAttempt = React.useRef(0)
  /** The named save or close under way, which the user may still cancel. */
  const inFlight = React.useRef<Cancellable | null>(null)

  const onConflict = (write: DocumentWrite) => {
    if (write.type !== "conflict") return false
    setConflict(true)
    setNotice(write.message)
    return true
  }

  /**
   * Resolves `false` when the text shown did not reach the store: refused,
   * failed, or in conflict. Only an acknowledged write counts; a failed one
   * is sent again by the next attempt.
   */
  const writeDraft = React.useCallback(async (): Promise<boolean> => {
    const current = latest.current
    // In conflict the text stays in the editor only (ADR-0016); a close under
    // way carries it itself.
    if (current.conflict) return false
    if (current.closing) return true
    const last = lastDraft.current
    if (last?.text === current.text && last.title === current.title)
      return pendingDraft.current?.sent === last
        ? pendingDraft.current.done
        : true
    // Only the last draft attempt speaks: an answer to an earlier one would
    // announce a state the editor has left (UX-SPEC § Autosauvegarde).
    const attempt = ++draftAttempt.current
    if (titleTooLong(current.title)) {
      setDraftNotice(`Draft not saved: ${TITLE_TOO_LONG}`)
      return false
    }
    revision.current += 1
    const sent = { title: current.title, text: current.text }
    lastDraft.current = sent
    setDraftNotice("Saving recovery draft…")
    const done = (async () => {
      try {
        const write = await library.saveDocument(newCommandId(), {
          document: current.document,
          revision: revision.current,
          title: sent.title,
          text: sent.text,
          connection,
          named: false,
        })
        if (write.type === "saved") void refreshLibrary(queryClient)
        if (latest.current.document !== current.document)
          return write.type !== "conflict"
        if (write.type === "conflict" && onConflict(write)) {
          setDraftNotice(`Draft not saved: ${write.message}`)
          return false
        }
        if (draftAttempt.current !== attempt) return true
        if (write.type === "saved")
          setDraftNotice(
            latest.current.text === sent.text &&
              latest.current.title === sent.title
              ? "Recovery draft saved locally."
              : "Newer edits are not in the recovery draft yet."
          )
        else if (write.type === "superseded")
          setDraftNotice("Draft replaced by a newer save of this query.")
        return true
      } catch (error) {
        // Not in the store: the next attempt must send this text again.
        if (lastDraft.current === sent) lastDraft.current = last
        if (draftAttempt.current === attempt)
          setDraftNotice(`Draft not saved: ${message(error)}`)
        return false
      }
    })()
    const pending = { sent, done }
    pendingDraft.current = pending
    try {
      return await done
    } finally {
      if (pendingDraft.current === pending) pendingDraft.current = null
    }
  }, [connection, queryClient])

  const debouncer = useDebouncer(() => void writeDraft(), {
    wait: DRAFT_IDLE_MS,
  })

  /**
   * Writes a pending draft now: before running, closing, on blur (ADR-0024).
   * Resolves `false` when the text shown is not in the store.
   */
  const flush = React.useCallback(async () => {
    debouncer.cancel()
    return writeDraft()
  }, [debouncer, writeDraft])

  // A seed never written is written like an edit, after the typing pause.
  React.useEffect(() => {
    if (lastDraft.current === null) debouncer.maybeExecute()
  }, [])

  const change = (next: { text?: string; title?: string }) => {
    if (next.text !== undefined) {
      setText(next.text)
      latest.current.text = next.text
    }
    if (next.title !== undefined) {
      setTitle(next.title)
      latest.current.title = next.title
    }
    if (!latest.current.conflict) debouncer.maybeExecute()
  }

  const unsaved = conflict || text !== savedText || title !== savedTitle

  const saveNamed = async (target: string) => {
    const current = latest.current
    if (titleTooLong(current.title) || current.title.trim() === "") {
      setProblem(true)
      setNotice(
        current.title.trim() === ""
          ? "A saved query needs a name."
          : TITLE_TOO_LONG
      )
      return false
    }
    debouncer.cancel()
    revision.current += 1
    const sent = { title: current.title, text: current.text }
    const before = lastDraft.current
    lastDraft.current = sent
    // Nothing reached the store: the recovery draft still has to catch up,
    // or a crash would offer the text from before the save.
    const unsent = () => {
      if (lastDraft.current === sent) lastDraft.current = before
      debouncer.maybeExecute()
    }
    const request = cancellable()
    inFlight.current = request
    setSaving(true)
    setProblem(false)
    setNotice(null)
    try {
      const write = await library.saveDocument(request.id, {
        document: target,
        revision: revision.current,
        title: sent.title,
        text: sent.text,
        connection,
        named: true,
      })
      if (onConflict(write)) return false
      if (write.type !== "saved") {
        unsent()
        if (request.cancelled) {
          setNotice("The save was cancelled before it was written.")
        } else {
          setProblem(true)
          setNotice(
            "The save was replaced by a newer one before it was written."
          )
        }
        return false
      }
      void refreshLibrary(queryClient)
      setConflict(false)
      setSavedText(sent.text)
      setSavedTitle(sent.title)
      setHasSavedCopy(true)
      setNotice(
        request.cancelled
          ? "Saved before the cancellation reached it."
          : latest.current.text === sent.text &&
              latest.current.title === sent.title
            ? "Saved in query library."
            : "Saved the requested version. Newer edits are not saved."
      )
      return true
    } catch (error) {
      unsent()
      setProblem(true)
      setNotice(message(error))
      return false
    } finally {
      if (inFlight.current === request) inFlight.current = null
      setSaving(false)
    }
  }

  /**
   * Cancels the named save or close under way. The answer still arrives: it
   * says whether the store had already committed.
   */
  const cancelWrite = () => {
    const request = inFlight.current
    if (!request || request.cancelled) return
    request.cancelled = true
    void backend.cancel(request.id).catch(() => undefined)
  }

  const save = async () => {
    if (latest.current.conflict || saving || closing) return false
    return saveNamed(latest.current.document)
  }

  /** Preserves the local text as a new document; the stored one is untouched. */
  const saveAsNew = async () => {
    if (saving || closing) return false
    void library.releaseDocument(latest.current.document).catch(() => undefined)
    const fresh = await library.newDocument()
    revision.current = 0
    latest.current = { ...latest.current, document: fresh, conflict: false }
    setDocument(fresh)
    setConflict(false)
    setHasSavedCopy(false)
    setSavedText("")
    setSavedTitle("")
    return saveNamed(fresh)
  }

  /**
   * Closes the working copy. Without a named copy the draft is discarded: the
   * caller asked the user first. In conflict nothing is written.
   */
  const close = async (discard: boolean) => {
    if (saving || closing) return false
    // Held from the start: a cancellation during the flush must stop the
    // close before it is sent, not be lost.
    const request = cancellable()
    inFlight.current = request
    if (!discard) await flush()
    debouncer.cancel()
    const current = latest.current
    if (isCancelled(request)) {
      if (inFlight.current === request) inFlight.current = null
      setNotice("Closing was cancelled. The console stays open.")
      return false
    }
    if (current.conflict) {
      if (inFlight.current === request) inFlight.current = null
      void library.releaseDocument(current.document).catch(() => undefined)
      return true
    }
    revision.current += 1
    setClosing(true)
    latest.current.closing = true
    setNotice("Closing the saved query…")
    try {
      const write = await library.closeDocument(
        request.id,
        current.document,
        revision.current,
        discard || !hasSavedCopy
      )
      // Committed before the cancellation reached it: the console goes.
      if (write.type === "closed") return true
      if (onConflict(write)) return false
      if (isCancelled(request) && write.type === "superseded") {
        setProblem(false)
        setNotice("Closing was cancelled. The console stays open.")
      } else {
        setProblem(true)
      }
      return false
    } catch (error) {
      setProblem(true)
      setNotice(message(error))
      return false
    } finally {
      if (inFlight.current === request) inFlight.current = null
      setClosing(false)
      latest.current.closing = false
    }
  }

  const saveState: SaveState = saving
    ? { status: "saving" }
    : conflict
      ? {
          status: "conflict",
          notice:
            notice ??
            "The stored query changed elsewhere. Save a new query to preserve both versions.",
        }
      : problem && notice !== null
        ? { status: "failed", notice }
        : {
            status: "idle",
            notice: notice ?? saveNotice({ hasSavedCopy, unsaved }),
          }

  return {
    document,
    title,
    text,
    unsaved,
    conflict,
    hasSavedCopy,
    saving,
    closing,
    draftNotice,
    saveState,
    titleError: titleTooLong(title) ? TITLE_TOO_LONG : null,
    fromAgent: seed.fromAgent,
    change,
    flush,
    save,
    saveAsNew,
    close,
    cancelWrite,
  }
}
