// What an action reads to decide whether it can run, and what it runs on
// (docs/adr/0041-registre-d-actions-menus-et-raccourcis.md, point 1).
//
// Screens own their state; they *publish* the part an action needs as a
// source: a small comparable `state` and stable `actions` that always call
// the screen's latest callbacks. The context changes when a state, the focus
// zone or a dialog changes — never on a keystroke that changes none of them.

import * as React from "react"
import { createStore } from "@tanstack/react-store"

import type { Zone } from "./manifest"
import { platform } from "./platform"
import type { Platform } from "./shortcut"

export interface WorkspaceState {
  activeTab: string | null
  tabCount: number
  consoleCount: number
  /** The active tab is an object with a preview grid. */
  objectActive: boolean
  /** A right column exists (inspector, assistant). */
  hasAside: boolean
  /** A destination for the assistant is declared (UX-SPEC). */
  hasAssistant: boolean
}

export interface WorkspaceActions {
  openConsole: () => void
  closeActiveTab: () => void
  nextTab: () => void
  previousTab: () => void
  showCatalog: () => void
  showLibrary: () => void
  focusPreview: () => void
  focusConsole: () => void
  toggleSidebar: () => void
  toggleAside: () => void
  openAssistant: () => void
  switchConnection: () => void
}

export interface ConsoleState {
  /** The session declares `SQL` and the console is not closing. */
  canRun: boolean
  running: boolean
  cancelling: boolean
  /** A named save or a close is being written. */
  writing: boolean
}

export interface ConsoleActions {
  run: () => void
  runAll: () => void
  explain: () => void
  cancel: () => void
  save: () => void
}

interface Source<TState, TActions> {
  state: TState
  actions: TActions
}

/** Every source a screen may publish. At most one of each at a time. */
export interface ActionSources {
  workspace?: Source<WorkspaceState, WorkspaceActions>
  console?: Source<ConsoleState, ConsoleActions>
  settings?: Source<Record<string, never>, { open: () => void }>
  navigation?: Source<Record<string, never>, { back: () => void }>
  /** The web menu bar of Windows and Linux. */
  menubar?: Source<Record<string, never>, { focus: () => void }>
}

type SourceKey = keyof ActionSources
type StateOf<TKey extends SourceKey> = NonNullable<ActionSources[TKey]>["state"]
type ActionsOf<TKey extends SourceKey> = NonNullable<
  ActionSources[TKey]
>["actions"]

interface Published {
  sources: ActionSources
  /** Who published each source: a late unmount must not remove its successor. */
  owners: Partial<Record<SourceKey, symbol>>
}

export const actionSources = createStore<Published>({
  sources: {},
  owners: {},
})

/**
 * Publishes `state` under `key` while mounted and non-null.
 *
 * The published entry changes only when `state` does (compared by value):
 * `actions` are wrapped once and always call the latest callbacks, so a
 * render that only changed a closure changes nothing downstream.
 */
export function useActionSource<TKey extends SourceKey>(
  key: TKey,
  state: StateOf<TKey> | null,
  actions: ActionsOf<TKey>
) {
  const owner = React.useRef(Symbol(key)).current
  const latest = React.useRef(actions)
  latest.current = actions
  const stable = React.useMemo(() => {
    const wrapped: Record<string, () => void> = {}
    for (const name of Object.keys(latest.current)) {
      wrapped[name] = () =>
        (latest.current as unknown as Record<string, () => void>)[name]?.()
    }
    return wrapped as unknown as ActionsOf<TKey>
  }, [])
  const stateKey = state === null ? null : JSON.stringify(state)
  React.useEffect(() => {
    if (stateKey === null) return
    const published = JSON.parse(stateKey) as StateOf<TKey>
    actionSources.setState((current) => ({
      sources: {
        ...current.sources,
        [key]: { state: published, actions: stable },
      },
      owners: { ...current.owners, [key]: owner },
    }))
    return () => {
      actionSources.setState((current) => {
        if (current.owners[key] !== owner) return current
        const sources = { ...current.sources }
        const owners = { ...current.owners }
        delete sources[key]
        delete owners[key]
        return { sources, owners }
      })
    }
  }, [key, stateKey, stable, owner])
}

/** Where the keyboard is, as the actions see it. */
export interface FocusState {
  /** The zones around the focus, innermost first. */
  zones: Array<Zone>
  /** A dialog holds the keyboard: only `app` actions remain. */
  modal: boolean
  /** The element focused last outside a menu, if still in the page. */
  focused: Element | null
}

export const NO_FOCUS: FocusState = { zones: [], modal: false, focused: null }

/**
 * The zones and the dialog around `element`.
 *
 * A dialog is any element with the `dialog` or `alertdialog` role, as Base UI
 * renders them: a shortcut behind it would be a click the user did not see.
 */
export function focusOf(element: Element | null): FocusState {
  if (element === null || !element.isConnected) return NO_FOCUS
  const zones: Array<Zone> = []
  for (
    let node: Element | null = element.closest("[data-action-zone]");
    node !== null;
    node = node.parentElement?.closest("[data-action-zone]") ?? null
  ) {
    const zone = node.getAttribute("data-action-zone")
    if (zone && isZone(zone) && !zones.includes(zone)) zones.push(zone)
  }
  const modal =
    element.closest('[role="dialog"], [role="alertdialog"]') !== null
  return { zones, modal, focused: element }
}

function isZone(value: string): value is Zone {
  return (
    value === "editor" ||
    value === "grid" ||
    value === "tabs" ||
    value === "tree" ||
    value === "assistant"
  )
}

/**
 * Whether focus on `element` belongs to a menu rather than to what the menu
 * acts on. Clicking the menu bar must not make `Find` forget the editor.
 */
export function isMenuFocus(element: Element | null) {
  return (
    element !== null &&
    element.closest('[role="menubar"], [role="menu"], [data-action-menu]') !==
      null
  )
}

export const focusStore = createStore<FocusState>(NO_FOCUS)

/** Follows the focus outside menus. Returns the function that stops it. */
export function trackFocus(): () => void {
  const update = (element: Element | null) => {
    if (isMenuFocus(element)) return
    const next = focusOf(element)
    const current = focusStore.state
    if (
      current.focused === next.focused &&
      current.modal === next.modal &&
      current.zones.join() === next.zones.join()
    )
      return
    focusStore.setState(() => next)
  }
  const onFocusIn = (event: FocusEvent) =>
    update(event.target instanceof Element ? event.target : null)
  const onFocusOut = (event: FocusEvent) => {
    if (event.relatedTarget !== null) return
    // Focus went to the body — a closed dialog, a removed element — or the
    // window lost it, which keeps `activeElement`.
    queueMicrotask(() => {
      if (document.activeElement === document.body) update(null)
    })
  }
  document.addEventListener("focusin", onFocusIn, true)
  document.addEventListener("focusout", onFocusOut, true)
  update(document.activeElement)
  return () => {
    document.removeEventListener("focusin", onFocusIn, true)
    document.removeEventListener("focusout", onFocusOut, true)
  }
}

export interface ActionContext extends FocusState {
  platform: Platform
  sources: ActionSources
}

/** The context now. A detached focused element counts as no focus. */
export function currentContext(): ActionContext {
  const focus = focusStore.state
  return {
    ...(focus.focused?.isConnected ? focus : NO_FOCUS),
    platform,
    sources: actionSources.state.sources,
  }
}

/** The element of `zone` around the focus, if any. */
export function zoneElement(context: ActionContext, zone: Zone) {
  return context.focused?.closest(`[data-action-zone="${zone}"]`) ?? null
}

/** What a zone element offers the actions that act inside it. */
export interface ZoneHandle {
  find?: () => void
  toggleComment?: () => void
}

const handles = new WeakMap<Element, ZoneHandle>()

/** Attaches `handle` to a zone element; `null` detaches it. */
export function setZoneHandle(element: Element, handle: ZoneHandle | null) {
  if (handle) handles.set(element, handle)
  else handles.delete(element)
}

/** The handle of the `zone` element around the focus. */
export function zoneHandle(
  context: ActionContext,
  zone: Zone
): ZoneHandle | null {
  const element = zoneElement(context, zone)
  return element ? (handles.get(element) ?? null) : null
}
