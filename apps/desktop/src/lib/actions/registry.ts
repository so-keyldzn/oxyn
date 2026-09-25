// The behavioural half of the registry
// (docs/adr/0041-registre-d-actions-menus-et-raccourcis.md, point 1): for
// each id of `actions.json`, when it can run and what it runs.
//
// Every trigger — keyboard, menu bar, native menu, button, and later the
// palette and the context menus — ends in `invoke`. An action runs what its
// button runs, through the screen that published it: nothing here reaches the
// backend except `request_exit`, which the button-less `File ▸ Exit` needs
// (I-01). No action takes a model's output, and none approves anything on
// `production`: those decisions stay with the backend's dialog (I-02, I-07).

import { recovery } from "@/lib/ipc/recovery"

import { currentContext, zoneElement, zoneHandle } from "./context"
import type { ActionContext, ConsoleState } from "./context"
import { actionSpec, onPlatform } from "./manifest"

/** `true`, a reason shown with the greyed entry, or not offered at all. */
export type Availability = true | { reason: string } | "absent"

export interface ActionBehaviour {
  enabled: (context: ActionContext) => Availability
  run: (context: ActionContext) => void | Promise<void>
}

const DIALOG_OPEN = { reason: "A dialog is open" }
const NO_WORKSPACE = { reason: "No connection is open" }
const NO_CONSOLE = { reason: "No console is active" }

/**
 * Whether a console action can run: the one condition the toolbar buttons
 * and every other trigger share (UX-SPEC, « Barre de menus » : « grisés dans
 * les mêmes cas que leurs boutons »).
 */
export function consoleAvailability(
  action: "run" | "cancel" | "save",
  state: ConsoleState
): Availability {
  switch (action) {
    case "run":
      if (state.running) return { reason: "A query is running" }
      if (!state.canRun)
        return { reason: "This console cannot run SQL right now" }
      return true
    case "cancel":
      if (!state.running) return { reason: "Nothing is running" }
      if (state.cancelling) return { reason: "Cancellation is under way" }
      return true
    case "save":
      if (state.writing) return { reason: "A save is under way" }
      return true
  }
}

/** Wraps a behaviour that needs the open workspace and no dialog. */
function inWorkspace(
  enabled: (
    workspace: NonNullable<ActionContext["sources"]["workspace"]>,
    context: ActionContext
  ) => Availability,
  run: (
    workspace: NonNullable<ActionContext["sources"]["workspace"]>,
    context: ActionContext
  ) => void
): ActionBehaviour {
  return {
    enabled: (context) => {
      if (context.modal) return DIALOG_OPEN
      const workspace = context.sources.workspace
      return workspace ? enabled(workspace, context) : NO_WORKSPACE
    },
    run: (context) => {
      const workspace = context.sources.workspace
      if (workspace) run(workspace, context)
    },
  }
}

function inConsole(
  action: "run" | "cancel" | "save",
  run: (console: NonNullable<ActionContext["sources"]["console"]>) => void
): ActionBehaviour {
  return {
    enabled: (context) => {
      if (context.modal) return DIALOG_OPEN
      // ⌘↵ or ⌘S in a stored query's read-only view runs and writes
      // nothing, not even the active console's (UX-SPEC, « Consultation
      // locale des requêtes »).
      if (
        action !== "cancel" &&
        zoneElement(context, "editor")?.hasAttribute("data-read-only")
      )
        return { reason: "This editor is read-only" }
      const console = context.sources.console
      return console ? consoleAvailability(action, console.state) : NO_CONSOLE
    },
    run: (context) => {
      const console = context.sources.console
      if (console) run(console)
    },
  }
}

/** The find field of the result around the focus. */
function resultFindField(context: ActionContext) {
  const field = zoneElement(context, "grid")?.querySelector("[data-find-field]")
  // Checked for presence first: the shell is prerendered in Node, which has
  // no `HTMLElement`.
  if (!field) return null
  return field instanceof HTMLElement ? field : null
}

/**
 * Runs an editing command of the webview on the element focused before the
 * menu opened. Windows and Linux only: macOS sends its own `copy:` and
 * `paste:` through the native Edit menu (ADR-0041, point 4).
 */
function editing(command: "undo" | "redo" | "cut" | "copy" | "selectAll") {
  return {
    enabled: () => true as const,
    run: (context: ActionContext) => {
      if (context.focused instanceof HTMLElement) context.focused.focus()
      // Deprecated but still the only way to reach a native field's own undo
      // stack and selection from outside a key press.
      document.execCommand(command)
    },
  } satisfies ActionBehaviour
}

export const behaviours: Record<string, ActionBehaviour> = {
  "app.settings": {
    enabled: (context) => {
      if (context.modal) return DIALOG_OPEN
      return context.sources.settings ? true : "absent"
    },
    run: (context) => context.sources.settings?.actions.open(),
  },
  // The same ordered exit as ⌘Q and the window's close button: drafts are
  // flushed and the close recorded before the process ends (ADR-0038). On
  // macOS the native Quit is handled in Rust; this path is the menu bar of
  // Windows and Linux, and the palette.
  "app.quit": {
    enabled: () => true,
    run: () =>
      recovery.requestExit().catch((error: unknown) => {
        console.error(`Could not ask for the exit: ${String(error)}`)
      }),
  },
  "console.new": inWorkspace(
    () => true,
    (workspace) => workspace.actions.openConsole()
  ),
  "connection.new": inWorkspace(
    () => true,
    (workspace) => workspace.actions.switchConnection()
  ),
  "library.open": inWorkspace(
    () => true,
    (workspace) => workspace.actions.showLibrary()
  ),
  "console.save": inConsole("save", (console) => console.actions.save()),
  "tab.close": inWorkspace(
    (workspace) =>
      workspace.state.activeTab === null ? { reason: "No tab is open" } : true,
    (workspace) => workspace.actions.closeActiveTab()
  ),
  "tab.next": inWorkspace(
    (workspace) =>
      workspace.state.consoleCount === 0
        ? { reason: "No console is open" }
        : true,
    (workspace) => workspace.actions.nextTab()
  ),
  "tab.previous": inWorkspace(
    (workspace) =>
      workspace.state.consoleCount === 0
        ? { reason: "No console is open" }
        : true,
    (workspace) => workspace.actions.previousTab()
  ),
  "edit.undo": editing("undo"),
  "edit.redo": editing("redo"),
  "edit.cut": editing("cut"),
  "edit.copy": editing("copy"),
  "edit.paste": {
    enabled: () => true,
    // `execCommand("paste")` is refused to pages by Chromium; the clipboard
    // API reads the text, which the focused field then inserts as typed.
    run: async (context) => {
      const target = context.focused
      const text = await navigator.clipboard.readText()
      if (target instanceof HTMLElement) target.focus()
      document.execCommand("insertText", false, text)
    },
  },
  "edit.selectAll": editing("selectAll"),
  // One action, searching where the focus is (ADR-0041, point 2).
  "edit.find": {
    enabled: (context) => {
      if (context.modal) return DIALOG_OPEN
      if (zoneHandle(context, "editor")?.find) return true
      if (resultFindField(context)) return true
      return { reason: "Focus the editor or a result to search it" }
    },
    run: (context) => {
      const editor = zoneHandle(context, "editor")?.find
      if (editor) return editor()
      const field = resultFindField(context)
      field?.focus()
      if (field instanceof HTMLInputElement) field.select()
    },
  },
  "editor.toggleComment": {
    enabled: (context) =>
      zoneHandle(context, "editor")?.toggleComment ? true : "absent",
    run: (context) => zoneHandle(context, "editor")?.toggleComment?.(),
  },
  // TODO(2026-12-31, débloqué par le lot 5 du plan « Interactions » : la
  // feuille des raccourcis) — declared now so that the zone rule of ⌘/ is
  // checked against Toggle comment.
  "help.shortcuts": {
    enabled: () => "absent",
    run: () => undefined,
  },
  "view.sidebar": inWorkspace(
    () => true,
    (workspace) => workspace.actions.toggleSidebar()
  ),
  "view.sidePanel": inWorkspace(
    (workspace) =>
      workspace.state.hasAside ? true : { reason: "No side panel here" },
    (workspace) => workspace.actions.toggleAside()
  ),
  // Without a declared destination the entry does not exist (UX-SPEC, « Le
  // workspace IA n'existe que s'il a été configuré »).
  "view.assistant": {
    enabled: (context) => {
      const workspace = context.sources.workspace
      if (!workspace?.state.hasAssistant) return "absent"
      return context.modal ? DIALOG_OPEN : true
    },
    run: (context) => context.sources.workspace?.actions.openAssistant(),
  },
  "view.catalog": inWorkspace(
    () => true,
    (workspace) => workspace.actions.showCatalog()
  ),
  "preview.focusGrid": inWorkspace(
    (workspace) =>
      workspace.state.objectActive
        ? true
        : { reason: "The active tab has no preview" },
    (workspace) => workspace.actions.focusPreview()
  ),
  "console.focus": inWorkspace(
    (workspace) =>
      workspace.state.consoleCount === 0
        ? { reason: "No console is open" }
        : true,
    (workspace) => workspace.actions.focusConsole()
  ),
  "console.run": inConsole("run", (console) => console.actions.run()),
  "console.runAll": inConsole("run", (console) => console.actions.runAll()),
  "console.explain": inConsole("run", (console) => console.actions.explain()),
  "console.cancel": inConsole("cancel", (console) => console.actions.cancel()),
  "nav.back": {
    enabled: (context) => {
      if (context.modal) return DIALOG_OPEN
      return context.sources.navigation ? true : "absent"
    },
    run: (context) => context.sources.navigation?.actions.back(),
  },
  "menubar.focus": {
    enabled: (context) => {
      if (context.modal) return DIALOG_OPEN
      return context.sources.menubar ? true : "absent"
    },
    run: (context) => context.sources.menubar?.actions.focus(),
  },
  // ⇧F10 and the Menu key: the focused element's own context menu, as a
  // right click would open it (the catalog tree already reads it this way).
  "context-menu.open": {
    enabled: (context) => (context.focused ? true : "absent"),
    run: (context) => {
      const target = context.focused
      if (!target) return
      const box = target.getBoundingClientRect()
      target.dispatchEvent(
        new MouseEvent("contextmenu", {
          bubbles: true,
          cancelable: true,
          clientX: box.left + Math.min(box.width / 2, 24),
          clientY: box.top + box.height / 2,
        })
      )
    },
  },
}

/** Where an invocation came from, for the trace of an ignored one. */
export type InvokeSource = "keyboard" | "menu" | "button" | "palette"

export function availability(
  id: string,
  context: ActionContext = currentContext()
): Availability {
  const spec = actionSpec(id)
  const behaviour = behaviours[id]
  if (!spec || !behaviour || !onPlatform(spec, context.platform))
    return "absent"
  return behaviour.enabled(context)
}

/**
 * Runs `id` if it can run **now**.
 *
 * A greyed menu entry is only a display: an activation that arrives after the
 * context changed — the native menu's click crosses an IPC hop — is
 * re-evaluated here, and ignored with a trace rather than run on a stale
 * state.
 */
export function invoke(
  id: string,
  source: InvokeSource,
  context: ActionContext = currentContext()
): boolean {
  const state = availability(id, context)
  if (state !== true) {
    console.info(
      `Ignored ${id} from the ${source}: ${state === "absent" ? "not offered here" : state.reason}`
    )
    return false
  }
  const behaviour = behaviours[id]
  if (!behaviour) return false
  const failed = (error: unknown) =>
    console.error(`${id} failed: ${String(error)}`)
  try {
    const running = behaviour.run(context)
    if (running instanceof Promise) running.catch(failed)
  } catch (error) {
    failed(error)
  }
  return true
}
