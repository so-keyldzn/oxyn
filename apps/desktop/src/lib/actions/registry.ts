// The behavioural half of the registry
// (docs/adr/0041-registre-d-actions-menus-et-raccourcis.md, point 1): for
// each id of `actions.json`, when it can run and what it runs.
//
// Every trigger — keyboard, menu bar, native menu, button, context menu and
// palette — ends in `invoke`. The context menus' own entries are in
// `menu-behaviours.ts`. An action runs what its
// button runs, through the screen that published it: nothing here reaches the
// backend except `request_exit`, which the button-less `File ▸ Exit` needs
// (I-01). No action takes a model's output, and none approves anything on
// `production`: those decisions stay with the backend's dialog (I-02, I-07).

import { recovery } from "@/lib/ipc/recovery"

import { consoleAvailability } from "./behaviour"
import type { ActionBehaviour, Availability } from "./behaviour"
import { currentContext, zoneElement, zoneHandle } from "./context"
import type {
  ActionContext,
  AppearanceActions,
  AppearanceState,
  OverlayActions,
} from "./context"
import { actionSpec, onPlatform } from "./manifest"
import { editorClipboardBehaviours, menuBehaviours } from "./menu-behaviours"

export { consoleAvailability }
export type { ActionBehaviour, Availability }

const DIALOG_OPEN = { reason: "A dialog is open" }
const NO_WORKSPACE = { reason: "No connection is open" }
const NO_CONSOLE = { reason: "No console is active" }

/**
 * The export control of the result on screen: the tab panel shown, and in it
 * the panel shown, down to the trigger (`components/oxyn/export-menu.tsx`).
 */
function visibleExport(): HTMLElement | null {
  if (typeof document === "undefined") return null
  for (const trigger of document.querySelectorAll("[data-action-export]")) {
    if (trigger instanceof HTMLElement && trigger.closest("[hidden]") === null)
      return trigger
  }
  return null
}

/** One entry of `View ▸ Text size` or `View ▸ Theme`. */
function appearanceChoice(
  pick: (actions: AppearanceActions) => void,
  isCurrent: (state: AppearanceState) => boolean
): ActionBehaviour {
  return {
    enabled: (context) => {
      if (context.modal) return DIALOG_OPEN
      return context.sources.appearance ? true : "absent"
    },
    run: (context) => {
      const appearance = context.sources.appearance
      if (appearance) pick(appearance.actions)
    },
    checked: (context) => {
      const appearance = context.sources.appearance
      return appearance ? isCurrent(appearance.state) : false
    },
  }
}

/** Opens one of the palette, the quick open or the shortcut sheet. */
function overlay(
  open: (actions: OverlayActions) => void,
  enabled: (context: ActionContext) => Availability = () => true
): ActionBehaviour {
  return {
    enabled: (context) => {
      if (context.modal) return DIALOG_OPEN
      if (!context.sources.overlays) return "absent"
      return enabled(context)
    },
    run: (context) => {
      const overlays = context.sources.overlays
      if (overlays) open(overlays.actions)
    },
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
 * menu opened. On macOS the native Edit menu sends its own `copy:` and
 * `paste:` (ADR-0041, point 4); this path serves the web bar and the context
 * menus.
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

/**
 * `fallback`, unless the editor's context menu is the target: CodeMirror
 * then cuts, copies and pastes itself, where `execCommand` would miss its
 * model.
 */
function withEditorMenu(
  action: "cut" | "copy" | "paste",
  fallback: ActionBehaviour
): ActionBehaviour {
  const editor = editorClipboardBehaviours[action]
  return {
    enabled: (context) =>
      context.sources.editorMenu
        ? editor.enabled(context)
        : fallback.enabled(context),
    run: (context) =>
      context.sources.editorMenu ? editor.run(context) : fallback.run(context),
  }
}

export const behaviours: Record<string, ActionBehaviour> = {
  ...menuBehaviours,
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
  // The tab a context menu names, else the active one.
  "tab.close": inWorkspace(
    (workspace, context) => {
      if (context.sources.tab)
        return context.sources.tab.actions.close ? true : "absent"
      return workspace.state.activeTab === null
        ? { reason: "No tab is open" }
        : true
    },
    (workspace, context) => {
      const close = context.sources.tab?.actions.close
      if (close) close()
      else workspace.actions.closeActiveTab()
    }
  ),
  // The last console closed comes back as it was written, never run (UX-SPEC,
  // « Menus contextuels », Onglet).
  "tab.reopen": inWorkspace(
    (workspace) => {
      if (!workspace.actions.reopenTab) return "absent"
      return workspace.state.reopenable
        ? true
        : { reason: "No closed console to reopen" }
    },
    (workspace) => workspace.actions.reopenTab?.()
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
  "edit.cut": withEditorMenu("cut", editing("cut")),
  "edit.copy": withEditorMenu("copy", editing("copy")),
  "edit.paste": withEditorMenu("paste", {
    enabled: () => true,
    // `execCommand("paste")` is refused to pages by Chromium; the clipboard
    // API reads the text, which the focused field then inserts as typed.
    run: async (context) => {
      const target = context.focused
      const text = await navigator.clipboard.readText()
      if (target instanceof HTMLElement) target.focus()
      document.execCommand("insertText", false, text)
    },
  }),
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
  // Outside the editor only: there, ⌘/ is Toggle comment (ADR-0041, point 2).
  "help.shortcuts": overlay((actions) => actions.openShortcuts()),
  // TODO(2026-12-31, débloqué par la commande open_external du plan
  // « Interactions », reportée du lot 1) — Oxyn opens no external link yet.
  "help.documentation": {
    enabled: () => ({
      reason: "Opening the documentation is not available yet",
    }),
    run: () => undefined,
  },
  "palette.open": overlay((actions) => actions.openPalette()),
  // The loaded catalog only, through its bounded search. In the `app` zone,
  // and greyed rather than absent, so that Ctrl+P never reaches the print of
  // WebView2 — not even from a dialog (ADR-0041, point 7).
  "object.quickOpen": overlay(
    (actions) => actions.openQuickOpen(),
    (context) => {
      if (!context.sources.workspace) return NO_WORKSPACE
      return context.sources.catalog
        ? true
        : { reason: "This source has no catalog to search" }
    }
  ),
  // TODO(2026-12-31, débloqué par le lot 7 du plan « Interactions » : le
  // multi-fenêtre d'ADR-0043) — one window for now.
  "window.new": {
    enabled: () => ({ reason: "Oxyn opens one window for now" }),
    run: () => undefined,
  },
  // What the export control of the result on screen offers, from the same
  // trigger: the formats, the save dialog and the reason of a greyed one
  // stay there (UX-SPEC, « Ce qui est exporté est ce qui est affiché »).
  "result.export": inWorkspace(
    () => {
      const trigger = visibleExport()
      if (!trigger) return { reason: "The active tab has no result to export" }
      const reason = trigger.getAttribute("data-action-export")
      return reason ? { reason } : true
    },
    () => visibleExport()?.click()
  ),
  "view.textSize.compact": appearanceChoice(
    (actions) => actions.setDensity("compact"),
    (state) => state.density === "compact"
  ),
  "view.textSize.comfortable": appearanceChoice(
    (actions) => actions.setDensity("comfortable"),
    (state) => state.density === "comfortable"
  ),
  "view.theme.light": appearanceChoice(
    (actions) => actions.setTheme("light"),
    (state) => state.theme === "light"
  ),
  "view.theme.dark": appearanceChoice(
    (actions) => actions.setTheme("dark"),
    (state) => state.theme === "dark"
  ),
  "view.theme.system": appearanceChoice(
    (actions) => actions.setTheme("system"),
    (state) => state.theme === "system"
  ),
  // TODO(2026-12-31, débloqué par le lot 9 du plan « Interactions » : le
  // formatage SQL, qui choisira un formateur par /versions) — no formatter
  // is shipped.
  "console.format": {
    enabled: () => ({ reason: "SQL formatting is not available yet" }),
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
export type InvokeSource =
  "keyboard" | "menu" | "button" | "palette" | "context-menu"

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

/** The check mark of `id`: `null` for an action that has none. */
export function checkedOf(
  id: string,
  context: ActionContext = currentContext()
): boolean | null {
  if (!actionSpec(id)?.check) return null
  return behaviours[id]?.checked?.(context) ?? false
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
