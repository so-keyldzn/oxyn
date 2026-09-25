import * as React from "react"
import { useStore } from "@tanstack/react-store"

import { visibleAssistantField } from "@/components/oxyn/assistant-composer"
import { AssistantEntryButton } from "@/components/oxyn/assistant-entry-button"
import { addressKey } from "@/components/oxyn/catalog-tree"
import type { OpenTarget } from "@/components/oxyn/catalog-tree"
import { CloseConsoleDialog } from "@/components/oxyn/close-console-dialog"
import { DEFINITION_WIDTH } from "@/components/oxyn/definition-beside"
import { StatusBar } from "@/components/oxyn/status-bar"
import { WorkspaceAside } from "@/components/oxyn/workspace-aside"
import type { AsideItem } from "@/components/oxyn/workspace-aside"
import { WorkspaceLayout } from "@/components/oxyn/workspace-layout"
import type { LeftView } from "@/components/oxyn/workspace-layout"
import { RenameConsoleDialog } from "@/components/oxyn/rename-console-dialog"
import { inOrder, moveItem } from "@/components/oxyn/pointer-drag"
import { WorkspaceTabs, tabPanelValue } from "@/components/oxyn/workspace-tabs"
import type {
  TabMenuTarget,
  WorkspaceTabItem,
} from "@/components/oxyn/workspace-tabs"
import { Button } from "@/components/ui/button"
import {
  Empty,
  EmptyContent,
  EmptyDescription,
  EmptyHeader,
  EmptyTitle,
} from "@/components/ui/empty"
import { Kbd, KbdGroup } from "@/components/ui/kbd"
import { TabsContent } from "@/components/ui/tabs"
import { toast } from "@/components/ui/toast"
import { answeringReach } from "@/features/assistant/availability"
import { usePinState } from "@/features/assistant/object-pin"
import { useAssistantAvailable } from "@/features/assistant/use-assistant-available"
import { ConsolePanel } from "@/features/consoles/console-panel"
import { flushAllDrafts } from "@/features/consoles/draft-registry"
import { useConsoles } from "@/features/consoles/use-consoles"
import { LibrarySidebar } from "@/features/library/library-sidebar"
import { RetainedResultTab } from "@/features/library/retained-result-tab"
import { inspectObject } from "@/features/metadata/inspection"
import { session, setSqlDraft } from "@/features/session"
import {
  forgetWorkspaceSessions,
  publishWorkspaceSessions,
} from "@/features/workspace/pending-transactions"
import {
  changePreferences,
  preferencesStore,
} from "@/features/settings/preferences"
import {
  CatalogSidebar,
  hasCatalog,
} from "@/features/workspace/catalog-sidebar"
import {
  savedObjectPlace,
  saveObjectPlace,
} from "@/features/workspace/object-location"
import { ObjectView } from "@/features/workspace/object-view"
import type { ObjectViewHandle } from "@/features/workspace/object-view"
import {
  objectOpenRequests,
  takeObjectOpenRequests,
} from "@/features/workspace/object-requests"
import {
  resultOpenRequests,
  takeResultOpenRequests,
} from "@/features/workspace/result-requests"
import { registerWorkspaceConsoles } from "@/features/windows/window-close"
import {
  publishWorkspaceDocuments,
  withdrawWorkspaceDocuments,
} from "@/features/windows/window-consoles"
import { useCompact } from "@/features/workspace/use-compact"
import { usePanelPreferences } from "@/features/workspace/use-panel-preferences"
import { useActionSource } from "@/lib/actions/context"
import { actionKeys } from "@/lib/actions/manifest"
import { BackendError } from "@/lib/ipc/client"
import { library } from "@/lib/ipc/library"
import { windows } from "@/lib/ipc/windows"
import type { HistoryRow } from "@/lib/ipc/library"
import type { SectionChoice } from "@/lib/ipc/location"
import type {
  CatalogAddress,
  CatalogNode,
  OpenConnection,
} from "@/lib/ipc/types"

export type { AsideItem } from "@/components/oxyn/workspace-aside"

interface ObjectTab {
  key: string
  node: CatalogNode
  target?: OpenTarget
  /** Brought back from the last session: nothing is read until asked. */
  restored?: SectionChoice
}

interface ResultTab {
  key: string
  connection: string
  result: string
  title: string
  /** The run ended without doubt. */
  succeeded: boolean
  /** The history run it came from; `null` for an agent's query. */
  history: HistoryRow | null
}

/**
 * Leaving a workspace: writes its drafts, hands the active text to the next
 * connection, and names the sessions left to close.
 */
export type WorkspaceExit = () => Promise<{ sessions: Array<string> }>

function objectKey(address: CatalogAddress) {
  return `object:${addressKey(address)}`
}

/** A relation reached by a foreign key: known by its address only. */
function relatedNode(address: CatalogAddress): CatalogNode {
  return {
    address,
    name: address.relation ?? address.namespace ?? "",
    kind: "table",
    holdsRecords: true,
    system: false,
    comment: null,
    loaded: false,
    stale: false,
    children: [],
  }
}

/**
 * The dense workbench (ADR-0011) for one open connection: explorer or library,
 * object, console and result tabs, right column and status bar.
 *
 * Every tab stays mounted while another is shown: a console keeps its editor
 * and its result, an object its preview. Hidden (`visible` false, while the
 * start screen is shown over it), the workspace keeps everything, takes no
 * shortcut, raises no dialog and reads no preview.
 *
 * Text meant for a console — from the assistant, say — goes through
 * `openInConsole` (`features/consoles/open-in-console.ts`): it lands in the
 * active console, unrun.
 */
export function WorkspaceScreen({
  open,
  visible = true,
  aside = [],
  onOpenSettings,
  onSwitchConnection,
  onDisconnect,
  exitRef,
}: {
  open: OpenConnection
  visible?: boolean
  /** Right-column panels (inspector, assistant). Hidden when empty. */
  aside?: Array<AsideItem>
  onOpenSettings?: () => void
  /** Shows the start screen; this workspace stays as it is. */
  onSwitchConnection: () => void
  /** Ends this workspace: drafts are written, sessions close. */
  onDisconnect: () => void
  /** Filled by the screen; called by whoever replaces this workspace. */
  exitRef?: React.Ref<WorkspaceExit>
}) {
  const compact = useCompact()
  const { sidebarOpen, setSidebarOpen, asideOpen, setAsideOpen } =
    usePanelPreferences(compact)
  const [leftView, setLeftView] = React.useState<LeftView>("catalog")
  const [asideActive, setAsideActive] = React.useState(aside[0]?.id ?? "")
  const inspectorWidth = useStore(
    preferencesStore,
    (state) => state.preferences.inspectorWidth
  )
  // `Ask AI` exists only when the assistant panel does: the column composes
  // itself from what is declared (docs/UX-SPEC.md).
  const hasAssistant = aside.some((item) => item.id === "assistant")
  const assistantEntry = useAssistantAvailable(open)
  // The destination the panel would send to: the tier label says where it
  // resolved, never where a provider is supposed to be.
  const { chosenKey } = usePinState(open.connection)
  const reach = answeringReach(assistantEntry, chosenKey)
  // Bumped by `Ask AI`: once the column is drawn, the question field takes
  // the focus. An effect, because the field is hidden until the render that
  // opens the column has been committed.
  const [focusAssistant, setFocusAssistant] = React.useState(0)
  React.useEffect(() => {
    if (focusAssistant > 0) visibleAssistantField()?.focus()
  }, [focusAssistant])
  const [objects, setObjects] = React.useState<Array<ObjectTab>>([])
  const [retained, setRetained] = React.useState<Array<ResultTab>>([])
  /** Result tabs with an export under way: they do not close meanwhile. */
  const [exporting, setExporting] = React.useState<ReadonlySet<string>>(
    () => new Set()
  )
  const [active, setActive] = React.useState<string | null>(null)
  // The tab shown, as a series of closes reads it between two awaits.
  const activeRef = React.useRef(active)
  activeRef.current = active
  const [tabOrder, setTabOrder] = React.useState<ReadonlyArray<string>>([])
  const lastConsole = React.useRef<string | null>(null)
  const objectsRef = React.useRef(objects)
  objectsRef.current = objects
  const objectHandles = React.useRef(new Map<string, ObjectViewHandle>())
  // One width for every object's definition, for as long as this workspace
  // is open; not yet kept across launches (ADR-0018).
  const [definitionWidth, setDefinitionWidth] = React.useState<number>(
    DEFINITION_WIDTH.initial
  )
  // The sub-view each object tab shows, reported by its view.
  const sections = React.useRef(new Map<string, SectionChoice>())

  // The object tab shown last is kept for the next launch, and forgotten
  // when the last one closes (UX-SPEC « L'état vit dans le workspace »).
  const keepPlace = React.useCallback(
    (tab: ObjectTab | undefined) =>
      void saveObjectPlace(
        open.connection,
        tab
          ? {
              address: tab.node.address,
              section: sections.current.get(tab.key) ?? "data",
            }
          : null
      ),
    [open.connection]
  )

  const activate = React.useCallback(
    (key: string) => {
      setActive(key)
      if (key.startsWith("console:")) lastConsole.current = key
      // A mounted object does not announce itself again: the inspector
      // follows the tab shown.
      const object = objectsRef.current.find((tab) => tab.key === key)
      if (object) {
        inspectObject({
          connection: open.connection,
          address: object.node.address,
        })
        keepPlace(object)
      } else inspectObject(null)
    },
    [open.connection, keepPlace]
  )

  // The object tab the last session left on this connection comes back as a
  // place: its view reads nothing until the user asks. Unless the recovery
  // screen was told otherwise, which keeps it saved all the same.
  React.useEffect(() => {
    if (session.state.objectPlaceDeclined) return
    let current = true
    void savedObjectPlace().then((saved) => {
      if (!current || saved?.connection !== open.connection) return
      const key = objectKey(saved.address)
      if (objectsRef.current.some((tab) => tab.key === key)) return
      sections.current.set(key, saved.section)
      setObjects((tabs) => [
        ...tabs,
        { key, node: relatedNode(saved.address), restored: saved.section },
      ])
      setActive((shown) => shown ?? key)
    })
    return () => {
      current = false
    }
  }, [open.connection])
  const work = useConsoles({ open, onActivate: activate })
  work.activeRef.current = lastConsole.current

  const asideItem = aside.find((item) => item.id === asideActive) ?? aside[0]

  const openObject = (node: CatalogNode, target?: OpenTarget) => {
    const key = objectKey(node.address)
    if (objectsRef.current.some((tab) => tab.key === key)) {
      activate(key)
      return
    }
    // A new ObjectView announces itself to the inspector when it mounts.
    setObjects((current) => [...current, { key, node, target }])
    setActive(key)
  }

  // Objects the assistant cites, opened from its sources as a click in the
  // catalog would; another connection's requests stay for its own screen.
  const objectRequests = useStore(objectOpenRequests)
  React.useEffect(() => {
    if (objectRequests.length === 0) return
    for (const request of takeObjectOpenRequests(open.connection)) {
      openObject(relatedNode(request.address))
    }
    // `openObject` reads the tabs through a ref: the requests are the trigger.
  }, [objectRequests, open.connection])

  const openResult = (tab: Omit<ResultTab, "key">) => {
    const key = `result:${tab.result}`
    setRetained((current) =>
      current.some((held) => held.key === key)
        ? current
        : [...current, { key, ...tab }]
    )
    activate(key)
  }

  // Results an agent's query left, opened from the assistant: the retained
  // rows, never a rerun. Another connection's requests stay for its screen.
  const resultRequests = useStore(resultOpenRequests)
  React.useEffect(() => {
    if (resultRequests.length === 0) return
    for (const request of takeResultOpenRequests(open.connection)) {
      openResult({
        connection: request.connection,
        result: request.result,
        title: "Agent result",
        // Reported completed by the executor before the panel offered it.
        succeeded: true,
        history: null,
      })
    }
    // `openResult` only sets state: the requests are the trigger.
  }, [resultRequests, open.connection])

  /**
   * Closes the tab `key`. Resolves `false` when a console's close was
   * cancelled or refused — what ends a series of closes — and `true`
   * otherwise, a result kept for its export included: the series goes on.
   */
  const closeTab = async (key: string): Promise<boolean> => {
    if (key.startsWith("console:")) return work.requestClose(key)
    // Closing would unmount the export and cancel its write (UX-SPEC
    // § Résultats conservés): the user ends or cancels it first.
    if (exporting.has(key)) {
      activate(key)
      toast.add({
        title: "Export in progress",
        description: "Finish or cancel the export before closing this result.",
        type: "warning",
      })
      return true
    }
    if (key.startsWith("object:")) {
      const remaining = objectsRef.current.filter((tab) => tab.key !== key)
      objectsRef.current = remaining
      sections.current.delete(key)
      keepPlace(remaining[remaining.length - 1])
    }
    setObjects((current) => current.filter((tab) => tab.key !== key))
    setRetained((current) => current.filter((tab) => tab.key !== key))
    if (activeRef.current !== key) return true
    // A console closed by the same series is no fallback.
    const live = (candidate: string | null | undefined) =>
      candidate && work.handles.current.has(candidate) ? candidate : null
    const fallback =
      live(lastConsole.current) ??
      live(work.entries.find((entry) => live(entry.key))?.key)
    if (fallback) activate(fallback)
    else setActive(null)
    activeRef.current = fallback
    return true
  }

  // `Close others`, `Close to the right`, `Close all`: one tab at a time,
  // each console asking as it would alone; Cancel ends the series there
  // (UX-SPEC, « Menus contextuels », Onglet). One series at a time.
  const closing = React.useRef(false)
  const closeSeries = async (keys: ReadonlyArray<string>) => {
    if (closing.current) return
    closing.current = true
    try {
      for (const key of keys) if (!(await closeTab(key))) return
    } finally {
      closing.current = false
    }
  }

  // The console whose name `Rename…` is changing.
  const [renaming, setRenaming] = React.useState<{
    key: string
    title: string
  } | null>(null)

  const focusConsole = () => {
    const key = lastConsole.current ?? work.entries[0]?.key
    if (!key) return
    activate(key)
    // Already mounted: focusing moves no editor.
    work.handles.current.get(key)?.focus()
  }

  const openHistoryCopy = async (row: HistoryRow) => {
    const entry = await library.readHistoryEntry(row.id).catch(() => null)
    if (entry) await work.openHistoryCopy(entry)
  }

  const entriesRef = React.useRef(work.entries)
  entriesRef.current = work.entries
  const handles = work.handles
  React.useImperativeHandle(
    exitRef,
    () => async () => {
      await flushAllDrafts()
      return {
        sessions: [
          open.session,
          ...entriesRef.current.flatMap((entry) =>
            entry.session ? [entry.session.session] : []
          ),
        ],
      }
    },
    [open.session, handles]
  )

  // The close of this window, when it is not the last, asks about every
  // workspace's consoles in one dialog, then closes them all (ADR-0043).
  const windowCloseRef = React.useRef({
    costs: work.windowCloseCosts,
    closeAll: work.closeAll,
  })
  windowCloseRef.current = {
    costs: work.windowCloseCosts,
    closeAll: work.closeAll,
  }
  React.useEffect(
    () =>
      registerWorkspaceConsoles(open.session, {
        connection: open.name,
        costs: () => windowCloseRef.current.costs(),
        closeAll: () => windowCloseRef.current.closeAll(),
      }),
    [open.session, open.name]
  )

  // Which consoles this window holds, for the next launch, which reopens them
  // here offline (ADR-0043). In tab order; the shown workspace's console in
  // front is the window's.
  const consoleDocuments = [...work.entries]
    .sort((a, b) => rank(tabOrder, a.key) - rank(tabOrder, b.key))
    .map((entry) => entry.seed.document)
  const activeDocument =
    work.entries.find((entry) => entry.key === active)?.seed.document ?? null
  const documentsKey = consoleDocuments.join("\n")
  React.useEffect(() => {
    publishWorkspaceDocuments(open.session, {
      documents: documentsKey === "" ? [] : documentsKey.split("\n"),
      active: activeDocument,
      visible,
    })
  }, [open.session, documentsKey, activeDocument, visible])
  React.useEffect(
    () => () => withdrawWorkspaceDocuments(open.session),
    [open.session]
  )

  // Hidden behind the start screen, the workspace offers its active text to
  // the next **new** connection, which copies it into its first console,
  // unrun. This console keeps it: a copy, never a move (UX-SPEC).
  const wasVisible = React.useRef(visible)
  React.useEffect(() => {
    if (wasVisible.current && !visible) {
      const text = lastConsole.current
        ? handles.current.get(lastConsole.current)?.text()
        : undefined
      // Disconnected, it keeps no console: the text moves, and the next
      // workspace does not claim an original that is gone.
      const kept = session.state.workspaces.some(
        (held) => held.session === open.session
      )
      if (text !== undefined) setSqlDraft(text, kept ? open.name : null)
    }
    wasVisible.current = visible
  }, [visible, open.name, handles])

  // The start screen names a hidden connection whose console holds a
  // transaction: it reads the sessions from here (ADR-0046).
  const heldSessions = React.useMemo(
    () =>
      work.entries.flatMap((entry) =>
        entry.session
          ? [
              {
                session: entry.session.session,
                transactions:
                  entry.session.capabilities.includes("TRANSACTIONS"),
              },
            ]
          : []
      ),
    [work.entries]
  )
  React.useEffect(() => {
    publishWorkspaceSessions(open.session, open.connection, heldSessions)
  }, [open.session, open.connection, heldSessions])
  React.useEffect(
    () => () => forgetWorkspaceSessions(open.session),
    [open.session]
  )

  const onConsole = active?.startsWith("console:") ?? false
  const cycleTab = (step: 1 | -1) => {
    const next = work.cycle(onConsole ? lastConsole.current : null, step)
    if (next) activate(next)
  }
  // Filled by the layout, which owns the compact sidebar's state.
  const toggleSidebar = React.useRef<(() => void) | null>(null)
  // What the registry's actions run here: the menu bar, the keyboard and the
  // palette reach the workspace through this, never around it (ADR-0041).
  // A hidden workspace publishes nothing, and a dialog — the close of a
  // console among them — holds the focus, which the registry reads as the
  // `modal` zone: a shortcut behind either is a click the user did not see.
  useActionSource(
    "workspace",
    visible
      ? {
          activeTab: active,
          tabCount: objects.length + work.entries.length + retained.length,
          consoleCount: work.entries.length,
          objectActive:
            active !== null && objects.some((tab) => tab.key === active),
          hasAside: aside.length > 0,
          hasAssistant,
          reopenable: work.reopenable,
        }
      : null,
    {
      openConsole: () => void work.openConsole(),
      closeActiveTab: () => {
        if (active) void closeTab(active)
      },
      reopenTab: () => void work.reopen(),
      nextTab: () => cycleTab(1),
      previousTab: () => cycleTab(-1),
      showCatalog: () => setLeftView("catalog"),
      showLibrary: () => setLeftView("library"),
      // The object on screen only: a console or a result has no preview.
      focusPreview: () => {
        if (active) objectHandles.current.get(active)?.focusPreview()
      },
      focusConsole,
      toggleSidebar: () => toggleSidebar.current?.(),
      toggleAside: () => {
        if (aside.length > 0) setAsideOpen(!asideOpen)
      },
      openAssistant: () => {
        setAsideActive("assistant")
        setAsideOpen(true)
        setFocusAssistant((count) => count + 1)
      },
      switchConnection: onSwitchConnection,
    }
  )
  // ⌘P searches the catalog this workspace has loaded, and opens a hit as a
  // click in the tree does.
  useActionSource(
    "catalog",
    visible && hasCatalog(open) ? { connection: open.connection } : null,
    {
      openObject: (object) =>
        openObject({ ...relatedNode(object.address), ...object, loaded: true }),
    }
  )

  const naturalTabs: Array<WorkspaceTabItem> = [
    ...objects.map((tab) => ({
      kind: "object" as const,
      key: tab.key,
      title: tab.node.name,
      objectKind: tab.node.kind,
    })),
    ...work.entries.map((entry) => ({
      kind: "console" as const,
      key: entry.key,
      title: work.meta[entry.key]?.title ?? entry.seed.title,
      fromAgent: work.meta[entry.key]?.fromAgent ?? entry.seed.fromAgent,
      activity: work.meta[entry.key]?.activity ?? "idle",
      unsaved: work.meta[entry.key]?.unsaved ?? false,
    })),
    ...retained.map((tab) => ({
      kind: "result" as const,
      key: tab.key,
      title: tab.title,
      exporting: exporting.has(tab.key),
    })),
  ]
  // The order the tabs were dragged into: display only, kept while the
  // workspace is open (UX-SPEC « Souris et glisser »).
  const tabs = inOrder(naturalTabs, (tab) => tab.key, tabOrder)
  const moveTab = (key: string, to: number) => {
    const keys = tabs.map((tab) => tab.key)
    setTabOrder(moveItem(keys, keys.indexOf(key), to))
  }
  /**
   * `Open in new window`: a console moves with its session; an object tab
   * moves as a place, which the new window opens without reading anything
   * until it is shown (ADR-0043).
   */
  const moveToWindow = async (key: string) => {
    if (key.startsWith("console:")) {
      await work.moveToNewWindow(key)
      return
    }
    const object = objectsRef.current.find((tab) => tab.key === key)
    if (!object) return
    try {
      await windows.openInNewWindow({
        type: "object",
        connection: open.connection,
        place: {
          address: object.node.address,
          section: sections.current.get(key) ?? "data",
        },
      })
    } catch (error) {
      toast.add({
        title: "The tab could not open in a new window",
        description:
          error instanceof BackendError ? error.message : String(error),
        type: "warning",
      })
      return
    }
    await closeTab(key)
  }

  /** Why a tab cannot move to a new window now, or `null`. */
  const moveBlocked = (key: string): string | null => {
    if (key.startsWith("object:")) return null
    const entry = work.entries.find((item) => item.key === key)
    const handle = work.handles.current.get(key)
    if (!entry || !handle)
      return "Only a console or an object opens in a new window"
    if (!entry.session) return "Attach the console to a connection first"
    const carried = handle.handoff()
    return "blocked" in carried ? carried.blocked : null
  }

  // The target of a tab's context menu: the same functions as the tab's
  // cross, ⌘W, the console's « Query name » field and the library switch
  // (I-01).
  const tabMenu = (key: string): TabMenuTarget => {
    const index = tabs.findIndex((tab) => tab.key === key)
    const others = tabs.filter((tab) => tab.key !== key).map((tab) => tab.key)
    const right = index < 0 ? [] : tabs.slice(index + 1).map((tab) => tab.key)
    const handle = work.handles.current.get(key)
    const isConsole = handle !== undefined
    const title = work.meta[key]?.title
    return {
      state: {
        console: isConsole,
        count: tabs.length,
        toTheRight: right.length,
        saved: handle?.closeReasons().hasSavedCopy ?? false,
        moveBlocked: moveBlocked(key),
      },
      actions: {
        close: () => void closeTab(key),
        closeOthers: () => void closeSeries(others),
        closeRight: () => void closeSeries(right),
        closeAll: () => void closeSeries(tabs.map((tab) => tab.key)),
        // Offered on every tab, so that an object's menu says why it
        // cannot: the registry greys them on `console`.
        duplicate: () => {
          if (isConsole) void work.duplicate(key)
        },
        rename: () => {
          if (isConsole) setRenaming({ key, title: title ?? "" })
        },
        // TODO(2026-12-31, débloqué par une sélection d'entrée exposée par
        // la bibliothèque) — the library opens; it cannot yet be told which
        // saved query to show.
        revealInLibrary: isConsole ? () => setLeftView("library") : undefined,
        openInNewWindow: () => void moveToWindow(key),
      },
    }
  }

  const activeObject = objects.find((tab) => tab.key === active)
  const activeEntry = work.entries.find(
    (entry) => entry.key === lastConsole.current
  )

  return (
    <>
      <WorkspaceLayout
        sidebar={
          leftView === "catalog" ? (
            <CatalogSidebar
              open={open}
              selected={
                activeObject ? addressKey(activeObject.node.address) : null
              }
              onSelect={(node) => openObject(node)}
              onOpen={(node, target) => openObject(node, target)}
              onNewConsole={(place) =>
                void work.openConsole({ context: place })
              }
              onLeave={onSwitchConnection}
            />
          ) : (
            <LibrarySidebar
              open={open}
              onLeave={onSwitchConnection}
              onOpenHistory={(entry) => void work.openHistoryCopy(entry)}
              onOpenResult={(row) => {
                if (row.connection !== null && row.result !== null)
                  openResult({
                    connection: row.connection,
                    result: row.result,
                    title: `Result #${row.id}`,
                    succeeded: row.status === "succeeded",
                    history: row,
                  })
              }}
              onOpenCopy={(document, entry) =>
                void work.openDocumentCopy(
                  document,
                  entry.connectionName ?? "No connection"
                )
              }
              onResume={(document) => void work.resume(document)}
            />
          )
        }
        sidebarOpen={sidebarOpen}
        onSidebarOpenChange={setSidebarOpen}
        sidebarToggleRef={toggleSidebar}
        leftView={leftView}
        onLeftViewChange={setLeftView}
        connectionName={open.name}
        environment={open.environment}
        readOnly={open.readOnly}
        tabs={
          <WorkspaceTabs
            tabs={tabs}
            active={active}
            opening={work.opening}
            onClose={(key) => void closeTab(key)}
            onMove={moveTab}
            onNewConsole={() => void work.openConsole()}
            onCancelOpening={work.cancelOpening}
            menuFor={tabMenu}
          />
        }
        aiEntry={
          hasAssistant ? (
            <AssistantEntryButton
              entry={assistantEntry}
              tier={open.privacyTier}
              reach={reach}
              pressed={asideOpen && asideActive === "assistant"}
              onPressedChange={(pressed) => {
                setAsideActive("assistant")
                setAsideOpen(pressed)
                if (pressed) setFocusAssistant((count) => count + 1)
              }}
            />
          ) : null
        }
        activeTab={active}
        onActiveTabChange={activate}
        notice={work.notice}
        aside={
          asideItem ? (
            <WorkspaceAside
              items={aside}
              active={asideItem.id}
              onActiveChange={setAsideActive}
              onClose={() => setAsideOpen(false)}
            />
          ) : null
        }
        asideOpen={asideOpen}
        onAsideOpenChange={setAsideOpen}
        asideWidth={inspectorWidth}
        onAsideWidthCommit={(width) =>
          void changePreferences({ inspectorWidth: width })
        }
        compact={compact}
        onOpenSettings={onOpenSettings}
        onDisconnect={onDisconnect}
        statusBar={
          <StatusBar
            connectionName={open.name}
            driver={open.driver}
            environment={open.environment}
            readOnly={open.readOnly}
            capabilities={
              activeEntry?.session?.capabilities ?? open.capabilities
            }
            execution={
              work.entries.length === 0 ? { status: "idle" } : work.summary
            }
          />
        }
      >
        {objects.map((tab) => (
          <TabsContent
            key={tab.key}
            value={tabPanelValue(tab.key)}
            keepMounted
            className="h-full min-h-0"
          >
            <ObjectView
              open={open}
              node={tab.node}
              active={visible && tab.key === active}
              initialTab={tab.target}
              handleRef={(handle) => {
                if (handle) objectHandles.current.set(tab.key, handle)
                else objectHandles.current.delete(tab.key)
              }}
              restored={tab.restored}
              onPlaceChange={(section) => {
                sections.current.set(tab.key, section)
                keepPlace(tab)
              }}
              // The value panel shows the selected row. At wide width the
              // column is where the preference left it; this action exists
              // only in the compact `Actions` menu, where it overlays.
              onInspectRow={
                aside.some((item) => item.id === "value")
                  ? () => {
                      setAsideActive("value")
                      setAsideOpen(true)
                    }
                  : undefined
              }
              onOpenRelated={(address) => openObject(relatedNode(address))}
              onOpenInConsole={(sql, title, options = {}) =>
                void work.openConsole({
                  text: sql,
                  title,
                  // Bound by the driver in the console, never concatenated
                  // into the statement (I-10).
                  parameters: options.parameters,
                  needsValues: options.needsValues,
                  notice: options.notice,
                })
              }
              definitionWidth={definitionWidth}
              onDefinitionWidthChange={setDefinitionWidth}
            />
          </TabsContent>
        ))}
        {work.entries.map((entry) => (
          // Kept mounted while another tab shows: the editor, its undo
          // history and a running query survive switching away.
          <TabsContent
            key={entry.key}
            value={tabPanelValue(entry.key)}
            keepMounted
            className="h-full min-h-0"
          >
            <ConsolePanel
              consoleKey={entry.key}
              open={open}
              session={entry.session}
              origin={entry.origin}
              attaching={work.attaching === entry.key}
              onAttach={() => void work.attach(entry.key)}
              onCancelAttach={work.cancelOpening}
              seed={entry.seed}
              initialNotice={entry.notice}
              initialResult={entry.initialResult ?? null}
              active={visible && entry.key === active}
              onMeta={work.onMeta}
              onSummary={work.setSummary}
              register={work.register}
            />
          </TabsContent>
        ))}
        {retained.map((tab) => (
          <TabsContent
            key={tab.key}
            value={tabPanelValue(tab.key)}
            keepMounted
            className="h-full min-h-0"
          >
            <RetainedResultTab
              connection={tab.connection}
              connectionName={open.name}
              result={tab.result}
              succeeded={tab.succeeded}
              // An agent's statement opens only with its provenance, from the
              // assistant itself (ADR-0023): no unmarked copy from here. A
              // write that needs inspection offers none either (I-13).
              onOpenCopy={
                tab.history && !tab.history.needsInspection
                  ? () => {
                      if (tab.history) void openHistoryCopy(tab.history)
                    }
                  : undefined
              }
              onExportingChange={(running) =>
                setExporting((current) => {
                  const next = new Set(current)
                  if (running) next.add(tab.key)
                  else next.delete(tab.key)
                  return next
                })
              }
            />
          </TabsContent>
        ))}
        {tabs.length === 0 ? (
          <Empty className="h-full border-0">
            <EmptyHeader>
              <EmptyTitle>No console open</EmptyTitle>
              <EmptyDescription>
                Open a console to write a query, or pick a table in the catalog.
              </EmptyDescription>
            </EmptyHeader>
            <EmptyContent>
              <Button size="sm" onClick={() => void work.openConsole()}>
                New console
                <KbdGroup>
                  {actionKeys("console.new").map((key) => (
                    <Kbd key={key}>{key}</Kbd>
                  ))}
                </KbdGroup>
              </Button>
            </EmptyContent>
          </Empty>
        ) : null}
      </WorkspaceLayout>

      <CloseConsoleDialog
        reasons={visible ? (work.closing?.reasons ?? null) : null}
        busy={work.closing?.busy ?? false}
        onCancel={() => void work.decideClose("cancel")}
        onSave={() => void work.decideClose("save")}
        onDiscard={() => void work.decideClose("discard")}
      />
      <RenameConsoleDialog
        title={visible ? (renaming?.title ?? null) : null}
        onCancel={() => setRenaming(null)}
        onRename={(title) => {
          if (renaming) work.handles.current.get(renaming.key)?.rename(title)
          setRenaming(null)
        }}
      />
    </>
  )
}

/** A tab's place in the strip; one not placed yet goes last. */
function rank(order: ReadonlyArray<string>, key: string) {
  const index = order.indexOf(key)
  return index === -1 ? order.length : index
}
