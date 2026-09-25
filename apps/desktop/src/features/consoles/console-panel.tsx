import * as React from "react"
import { keepPreviousData, useQuery } from "@tanstack/react-query"
import { useStore } from "@tanstack/react-store"
import type { EditorView } from "@codemirror/view"

import { ApprovalDialog } from "@/components/oxyn/approval-dialog"
import { ConsoleToolbar } from "@/components/oxyn/console-toolbar"
import { ConsoleView } from "@/components/oxyn/console-view"
import { ParameterEditor } from "@/components/oxyn/parameter-editor"
import type { ParameterRow } from "@/components/oxyn/parameter-editor"
import { ResultFindBar } from "@/components/oxyn/result-find-bar"
import type { FindDirection } from "@/components/oxyn/result-find-bar"
import type { GridPosition } from "@/components/oxyn/grid-selection"
import { ResultPanel } from "@/components/oxyn/result-panel"
import { OfflineConsoleBar } from "@/components/oxyn/offline-console-bar"
import { SessionContextPicker } from "@/components/oxyn/session-context-picker"
import type { SessionContextState } from "@/components/oxyn/session-context-picker"
import { SqlEditor, targetOf } from "@/components/oxyn/sql-editor"
import type { EditorTarget } from "@/components/oxyn/sql-editor"
import type { ExecutionSummary } from "@/components/oxyn/status-bar"
import { transactionNotice } from "@/features/consoles/console-model"
import type {
  CloseReasons,
  ConsoleActivity,
} from "@/features/consoles/console-model"
import { registerDraftFlush } from "@/features/consoles/draft-registry"
import { useConsoleDocument } from "@/features/consoles/use-console-document"
import type { ConsoleSeed } from "@/features/consoles/use-console-document"
import { releaseResult } from "@/features/metadata/inspection"
import { useResultFind } from "@/features/metadata/use-result-find"
import {
  ValueInspectionDialog,
  gridInspection,
  useReleaseSelection,
} from "@/features/metadata/value-inspection"
import { useResultDensity } from "@/features/settings/use-result-density"
import { ExportMenu } from "@/features/workspace/export-menu"
import { useExecution } from "@/features/workspace/use-execution"
import { BackendError, backend, newCommandId } from "@/lib/ipc/client"
import { consoles } from "@/lib/ipc/consoles"
import type {
  ConsoleSession,
  ParameterRefusal,
  RunTarget,
  SessionPlace,
} from "@/lib/ipc/consoles"
import { catalogVersion, transactionStates } from "@/lib/ipc/events"
import { results } from "@/lib/ipc/results"
import type { OpenConnection, ResultColumn } from "@/lib/ipc/types"

/** What the workspace may ask of a console it does not render itself. */
export interface ConsoleHandle {
  closeReasons: () => CloseReasons
  /** The text as typed, to carry into the next connection's first console. */
  text: () => string
  /** Writes the pending draft now; `false` if the text is not in the store. */
  flush: () => Promise<boolean>
  /** Saves the named copy — or a new copy in conflict. */
  save: () => Promise<boolean>
  /** Closes the document and cancels what runs. Resolves `false` if it stayed. */
  close: (discard: boolean) => Promise<boolean>
  /** Cancels the named save or close under way; its answer still resolves. */
  cancelWrite: () => void
  /** Puts text at the cursor, replacing the selection. Never runs it. */
  insert: (sql: string, notice: string | null) => void
  focus: () => void
}

export interface ConsoleMeta {
  title: string
  fromAgent: boolean
  activity: ConsoleActivity
  unsaved: boolean
}

/** An offline console declares nothing: it runs nothing. */
const NO_CAPABILITIES: Array<string> = []

/** Stable, so the inspectors are not rebuilt while the columns are unknown. */
const NO_COLUMNS: Array<ResultColumn> = []

function place(state: SessionContextState): SessionPlace | null {
  if (state.status === "declared") return state.place
  if (state.status === "serverDefault") return null
  return state.current
}

/**
 * One console: its session, its document, its runs (ADR-0015).
 *
 * Stays mounted while hidden, so an answer always reaches the console that
 * asked. A pending approval is shown only while the console is visible and
 * stays attached to it when another tab is selected.
 */
export function ConsolePanel({
  consoleKey,
  open,
  session,
  origin,
  attaching,
  onAttach,
  onCancelAttach,
  seed,
  initialNotice,
  active,
  onMeta,
  onSummary,
  register,
}: {
  consoleKey: string
  open: OpenConnection
  /** `null` while offline: the console edits and saves, and runs nothing. */
  session: ConsoleSession | null
  /** The connection the document is saved under; kept while offline. */
  origin: string | null
  /** An offline console is being given a session. */
  attaching: boolean
  onAttach: () => void
  onCancelAttach: () => void
  seed: ConsoleSeed
  initialNotice: string | null
  active: boolean
  onMeta: (key: string, meta: ConsoleMeta) => void
  onSummary: (summary: ExecutionSummary) => void
  register: (key: string, handle: ConsoleHandle | null) => void
}) {
  const doc = useConsoleDocument({ seed, connection: origin })
  const capabilities = session?.capabilities ?? NO_CAPABILITIES
  const execution = useExecution({
    serverCancel: capabilities.includes("SERVER_SIDE_CANCEL"),
  })
  // Seeded values (« Related rows », say) are bound, never written into the
  // SQL: the console shows them in its Parameters panel, already open.
  const [rows, setRows] = React.useState<Array<ParameterRow>>(() =>
    seed.parameters.map((parameter) => ({
      id: crypto.randomUUID(),
      type: parameter.type,
      text: parameter.text,
    }))
  )
  const [parametersOpen, setParametersOpen] = React.useState(
    seed.parameters.length > 0 || seed.needsValues
  )
  const [parameterError, setParameterError] =
    React.useState<ParameterRefusal | null>(null)
  const [target, setTarget] = React.useState<EditorTarget["kind"]>("statement")
  const [notice, setNotice] = React.useState(
    seed.needsValues
      ? (initialNotice ??
          "A value is still missing: fill it in Parameters before running.")
      : initialNotice
  )
  const [context, setContext] = React.useState<SessionContextState>({
    status: "serverDefault",
  })
  const contextRun = React.useRef<string | null>(null)
  // Bumped on every `setRows`, so `submit` can tell a stale
  // `validateParameters` answer from a current one.
  const rowsGeneration = React.useRef(0)
  const view = React.useRef<EditorView | null>(null)

  const canRun = capabilities.includes("SQL") && !doc.closing
  // The last state the session reported — never one guessed from a `BEGIN`
  // being submitted (ADR-0039 §5).
  const declaresTransactions = capabilities.includes("TRANSACTIONS")
  const reported = useStore(transactionStates, (all) =>
    session ? all[session.session] : undefined
  )
  const transaction = session
    ? transactionNotice(reported, declaresTransactions)
    : null
  const withContext = capabilities.includes("SESSION_CONTEXT")
  const version = useStore(catalogVersion)
  const choices = useQuery({
    queryKey: ["session-context-choices", open.connection, version],
    queryFn: () => consoles.contextChoices(open.connection),
    enabled: withContext,
    // A catalog refresh bumps `version`, replacing this key: without this,
    // `choices.data` drops to `undefined` mid-review and the picker briefly
    // reports "no schema" for a connection it already knows about.
    placeholderData: keepPreviousData,
  })

  // Whether the run whose result is shown was an `Explain`.
  const [explained, setExplained] = React.useState(false)
  const submitting = React.useRef(false)
  const run = async (runTarget: RunTarget, explain = false) => {
    // A double click submits once: the parameter check awaits before start.
    if (!canRun || execution.running || submitting.current) return
    submitting.current = true
    try {
      await submit(runTarget, explain)
    } finally {
      submitting.current = false
    }
  }

  const submit = async (runTarget: RunTarget, explain: boolean) => {
    // Immediate write (ADR-0024): what runs is what is saved.
    void doc.flush()
    const parameters = rows.map((row) => ({ type: row.type, text: row.text }))
    if (parameters.length > 0) {
      // `validateParameters` is a pure, synchronous-on-the-Rust-side command
      // with no command id (nothing for `backend.cancel` to target): the
      // only way to tell a stale answer from a current one is to compare the
      // rows generation captured before the call to the one now current.
      const generation = rowsGeneration.current
      let refused: ParameterRefusal | null
      try {
        refused = await consoles.validateParameters(parameters)
      } catch (error: unknown) {
        // The failure is on the IPC channel, not on the values: it must
        // surface even if the rows changed meanwhile, and it must not leave
        // a previous refusal displayed for values that changed since.
        setParameterError(null)
        setNotice(String(error))
        return
      }
      if (rowsGeneration.current !== generation) {
        // The answer is about values no longer shown (UX-SPEC « Consoles
        // indépendantes » : a stale answer does not act). Starting the run
        // with the old snapshot would execute something other than what the
        // person sees.
        return
      }
      setParameterError(refused)
      if (refused !== null) {
        setParametersOpen(true)
        return
      }
    }
    if (!session) return
    setNotice(null)
    // Known here and nowhere else: the rows of a plan look like any others.
    setExplained(explain)
    execution.start(
      (id) =>
        consoles.run(id, open.connection, session.session, {
          sql: doc.text,
          target: runTarget,
          parameters,
          explain,
        }),
      { session: declaresTransactions ? session.session : null }
    )
  }

  const targetNow = (): RunTarget =>
    view.current ? targetOf(view.current) : { kind: "all" }

  const chooseContext = (next: SessionPlace) => {
    if (!session) return
    if (contextRun.current) void backend.cancel(contextRun.current)
    const id = newCommandId()
    contextRun.current = id
    const current = place(context)
    setContext({
      status: "changing",
      target: next.namespace ?? "Server default",
      current,
    })
    consoles.setContext(id, open.connection, session.session, next).then(
      (outcome) => {
        if (contextRun.current !== id) return
        contextRun.current = null
        if (outcome.type === "cancelled") {
          setContext(
            current
              ? { status: "declared", place: current }
              : { status: "serverDefault" }
          )
          setNotice(
            "Context change cancelled. This session still resolves where it did."
          )
          return
        }
        setContext(
          outcome.place?.namespace
            ? { status: "declared", place: outcome.place }
            : { status: "serverDefault" }
        )
      },
      (error: unknown) => {
        if (contextRun.current !== id) return
        contextRun.current = null
        setContext({
          status: "failed",
          message: error instanceof Error ? error.message : String(error),
          retryable: error instanceof BackendError && error.retryable,
          current,
        })
      }
    )
  }

  const activity: ConsoleActivity = doc.closing
    ? "closing"
    : doc.saving
      ? "saving"
      : execution.approval
        ? "approval"
        : execution.running
          ? "running"
          : "idle"

  React.useEffect(() => {
    onMeta(consoleKey, {
      title: doc.title,
      fromAgent: doc.fromAgent,
      activity,
      unsaved: doc.unsaved,
    })
  }, [consoleKey, doc.title, doc.fromAgent, activity, doc.unsaved, onMeta])

  React.useEffect(() => {
    if (active) onSummary(execution.summary)
  }, [active, execution.summary, onSummary])

  // The workspace reaches the latest state through one stable handle.
  const latest = React.useRef({
    doc,
    execution,
    transaction,
    connection: open.name,
    run: () => run(targetNow()),
  })
  latest.current = {
    doc,
    execution,
    transaction,
    connection: open.name,
    run: () => run(targetNow()),
  }
  React.useEffect(() => {
    const handle: ConsoleHandle = {
      closeReasons: () => ({
        title: latest.current.doc.title || "Untitled query",
        connection: latest.current.connection,
        conflict: latest.current.doc.conflict,
        hasSavedCopy: latest.current.doc.hasSavedCopy,
        unsaved: latest.current.doc.unsaved,
        running: latest.current.execution.running,
        transaction: latest.current.transaction,
      }),
      text: () => latest.current.doc.text,
      flush: () => latest.current.doc.flush(),
      save: () =>
        latest.current.doc.conflict
          ? latest.current.doc.saveAsNew()
          : latest.current.doc.save(),
      close: async (discard) => {
        const closed = await latest.current.doc.close(discard)
        if (closed) latest.current.execution.reset()
        return closed
      },
      cancelWrite: () => latest.current.doc.cancelWrite(),
      insert: (sql, why) => {
        const editor = view.current
        if (editor) {
          const range = editor.state.selection.main
          editor.dispatch({
            changes: { from: range.from, to: range.to, insert: sql },
            selection: { anchor: range.from + sql.length },
          })
          editor.focus()
        } else {
          latest.current.doc.change({ text: latest.current.doc.text + sql })
        }
        setNotice(why ?? "Text placed in this console. Nothing was executed.")
      },
      focus: () => view.current?.focus(),
    }
    register(consoleKey, handle)
    const unregister = registerDraftFlush(consoleKey, handle.flush)
    return () => {
      unregister()
      register(consoleKey, null)
    }
  }, [consoleKey, register])

  const state = execution.state
  const exportable =
    state.status === "populated" &&
    state.complete &&
    !state.truncated &&
    !state.cancelled

  // Stable callbacks: the results only re-render when the result changes, not
  // on every key typed in the editor (PERFORMANCE, interaction budgets).
  // The result is addressable while the statement still streams, so the find
  // bar and the inspectors work on the rows already received.
  const resultId =
    state.status === "populated"
      ? state.result
      : state.status === "running"
        ? (state.result ?? null)
        : null
  const resultColumns =
    state.status === "populated"
      ? state.columns
      : state.status === "running"
        ? (state.columns ?? NO_COLUMNS)
        : NO_COLUMNS
  const fetchPage = React.useCallback(
    (offset: number, limit: number) =>
      resultId
        ? // `readResultPage` answers « expired » rather than failing when the
          // buffer was released under the grid.
          results.readResultPage(open.connection, resultId, offset, limit)
        : Promise.reject(new Error("No result")),
    [open.connection, resultId]
  )
  const cancel = React.useCallback(() => latest.current.execution.cancel(), [])
  const retry = React.useCallback(() => void latest.current.run(), [])

  const find = useResultFind(resultId)
  // The hook rebuilds its callbacks on every render; the result area must not
  // re-render on every keystroke in the editor (PERFORMANCE), so the console
  // hands the panel stable ones.
  const latestFind = React.useRef(find)
  latestFind.current = find
  const onFind = React.useCallback(
    (needle: string, direction: FindDirection) =>
      void latestFind.current.find(needle, direction),
    []
  )
  const onClearFind = React.useCallback(() => latestFind.current.clear(), [])
  const source = `console:${consoleKey}`
  useReleaseSelection(source)
  // A result this console no longer shows leaves the inspectors with it.
  const shownResult = React.useRef<string | null>(null)
  React.useEffect(() => {
    const previous = shownResult.current
    shownResult.current = resultId
    if (previous && previous !== resultId) releaseResult(previous)
  }, [resultId])
  React.useEffect(
    () => () => {
      if (shownResult.current) releaseResult(shownResult.current)
    },
    []
  )

  const exportMenu = React.useMemo(
    () => (
      <ExportMenu
        connection={open.connection}
        result={resultId}
        exportable={exportable}
        reason="Only a complete result can be exported."
      />
    ),
    [open.connection, resultId, exportable]
  )
  const inspection = React.useMemo(
    () =>
      gridInspection({
        source,
        open,
        result: resultId,
        columns: resultColumns,
      }),
    [source, open, resultId, resultColumns]
  )
  const statement = doc.text
  const findBar = React.useMemo(
    () => (
      <ResultFindBar
        answer={find.answer}
        searching={find.searching}
        error={find.error}
        onFind={onFind}
        onClear={onClearFind}
      />
    ),
    [find.answer, find.searching, find.error, onFind, onClearFind]
  )
  const editQuery = React.useCallback(() => view.current?.focus(), [])
  const density = useResultDensity()
  const latestInspection = React.useRef(inspection)
  latestInspection.current = inspection
  const onActiveChange = React.useCallback((position: GridPosition | null) => {
    // « Next match » starts from where the user is in the grid.
    latestFind.current.trackRow(position?.row ?? 0)
    latestInspection.current.onActiveChange(position)
  }, [])
  const resultPanel = React.useMemo(
    () => (
      <ResultPanel
        state={state}
        fetchPage={fetchPage}
        onCancel={cancel}
        onRetry={retry}
        toolbar={resultId ? findBar : undefined}
        footerActions={exportMenu}
        density={density}
        context={{ connectionName: open.name, statement }}
        onEditQuery={editQuery}
        matches={find.matches}
        reveal={find.reveal}
        onInspect={inspection.onInspect}
        onActiveChange={onActiveChange}
        plan={explained}
      />
    ),
    [
      explained,
      state,
      fetchPage,
      cancel,
      retry,
      resultId,
      findBar,
      exportMenu,
      density,
      open.name,
      statement,
      editQuery,
      find.matches,
      find.reveal,
      inspection.onInspect,
      onActiveChange,
    ]
  )
  const elapsedMs = useElapsed(active ? execution.startedAt : null)

  return (
    <>
      <ConsoleView
        toolbar={
          <ConsoleToolbar
            running={execution.running}
            cancelling={execution.cancelling}
            elapsedMs={elapsedMs}
            canRun={canRun}
            readOnly={session?.readOnly ?? false}
            target={target}
            onRun={() => void run(targetNow())}
            onRunAll={() => void run({ kind: "all" })}
            onExplain={() => void run(targetNow(), true)}
            onCancel={execution.cancel}
            parameterCount={rows.length}
            parametersOpen={parametersOpen}
            onToggleParameters={() => setParametersOpen((value) => !value)}
            title={doc.title}
            onTitleChange={(title) => {
              if (!doc.closing) doc.change({ title })
            }}
            titleError={doc.titleError}
            save={doc.saveState}
            closing={doc.closing}
            onCancelWrite={doc.cancelWrite}
            onSave={() => void (doc.conflict ? undefined : doc.save())}
            onSaveAsNew={() => void doc.saveAsNew()}
            transaction={transaction}
            context={
              withContext ? (
                <SessionContextPicker
                  connectionName={open.name}
                  choices={choices.data ?? [{ catalog: null, namespace: null }]}
                  state={context}
                  onChoose={chooseContext}
                  onCancel={() => {
                    if (contextRun.current)
                      void backend.cancel(contextRun.current)
                  }}
                  onLoadSchemas={() =>
                    void backend
                      .refreshCatalog(
                        newCommandId(),
                        open.connection,
                        open.session,
                        null
                      )
                      .catch(() => undefined)
                  }
                />
              ) : null
            }
          />
        }
        offline={
          session === null ? (
            <OfflineConsoleBar
              connectionName={open.name}
              sameConnection={origin === open.connection}
              attaching={attaching}
              onAttach={onAttach}
              onCancel={onCancelAttach}
            />
          ) : null
        }
        notice={notice}
        draftNotice={doc.draftNotice}
        parameters={
          parametersOpen ? (
            <ParameterEditor
              rows={rows}
              onChange={(next) => {
                rowsGeneration.current += 1
                setRows(next)
                setParameterError(null)
              }}
              refusal={parameterError}
            />
          ) : null
        }
        editor={
          <SqlEditor
            value={doc.text}
            onChange={(text) => doc.change({ text })}
            driver={open.driver}
            running={execution.running}
            readOnly={doc.closing}
            onRun={(editorTarget) => void run(editorTarget)}
            onRunAll={() => void run({ kind: "all" })}
            onSave={() => void (doc.conflict ? undefined : doc.save())}
            onCancel={execution.cancel}
            onTargetChange={setTarget}
            onBlur={() => void doc.flush()}
            onEditor={(editor) => {
              view.current = editor
            }}
            autoFocus={active}
          />
        }
        results={resultPanel}
      />
      <ValueInspectionDialog source={source} />
      {active ? (
        <ApprovalDialog
          approval={execution.approval}
          connectionName={open.name}
          environment={open.environment}
          deciding={execution.deciding}
          onDecide={execution.decide}
        />
      ) : null}
    </>
  )
}

/** Milliseconds since `since`, refreshed four times a second while set. */
function useElapsed(since: number | null) {
  const [now, setNow] = React.useState(() => Date.now())
  React.useEffect(() => {
    if (since === null) return
    setNow(Date.now())
    const timer = window.setInterval(() => setNow(Date.now()), 250)
    return () => window.clearInterval(timer)
  }, [since])
  return since === null ? null : now - since
}
