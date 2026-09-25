// The one `keydown` listener of the application
// (docs/adr/0041-registre-d-actions-menus-et-raccourcis.md, point 3), in the
// capture phase on `window`: it sees a combination before CodeMirror, the
// sidebar or any component does, and resolves it by focus zone.

import { createStore } from "@tanstack/react-store"

import { currentContext, focusOf } from "./context"
import type { ActionContext } from "./context"
import { chordsOf, manifest, onPlatform, placementOf } from "./manifest"
import type { ActionSpec, Manifest, Zone } from "./manifest"
import { nativeMenu, platform } from "./platform"
import { availability, invoke } from "./registry"
import { isModifierKey, keyOf, matches } from "./shortcut"
import type { Chord, Platform } from "./shortcut"

interface Binding {
  id: string
  chord: Chord
  binding: ActionSpec["binding"]
}

/** The combinations of each zone on `platform`, in manifest order. */
export function bindingsByZone(
  source: Manifest,
  on: Platform
): Map<Zone, Array<Binding>> {
  const zones = new Map<Zone, Array<Binding>>()
  for (const spec of source.actions) {
    if (!onPlatform(spec, on)) continue
    for (const chord of chordsOf(spec, on)) {
      const list = zones.get(spec.zone) ?? []
      list.push({ id: spec.id, chord, binding: spec.binding })
      zones.set(spec.zone, list)
    }
  }
  return zones
}

/**
 * The actions the native macOS bar binds itself: placed in it, bound by the
 * dispatcher, and never on `Escape` (ADR-0041, point 4 — Cancel would take
 * Escape from every dialog and completion).
 */
export function nativeAccelerators(source: Manifest): Set<string> {
  const ids = new Set<string>()
  for (const spec of source.actions) {
    if (!onPlatform(spec, "mac") || spec.binding !== "dispatcher") continue
    if (placementOf(spec, "mac") === null) continue
    const chords = chordsOf(spec, "mac")
    if (chords.length > 0 && chords[0]?.key !== "Escape") ids.add(spec.id)
  }
  return ids
}

/** The action a key press means in `context`, or `null`. */
export function resolve(
  event: KeyboardEvent,
  context: Pick<ActionContext, "zones" | "modal">,
  bindings: Map<Zone, Array<Binding>>
): Binding | null {
  // The most precise zone first; a dialog leaves only `app`.
  const scopes: Array<Zone> = context.modal
    ? ["app"]
    : [...context.zones, "global", "app"]
  for (const zone of scopes) {
    const found = bindings
      .get(zone)
      ?.find((binding) => matches(binding.chord, event))
    if (found) return found
  }
  return null
}

function isTextField(target: EventTarget | null) {
  if (!(target instanceof HTMLElement)) return false
  return (
    target.isContentEditable ||
    target instanceof HTMLTextAreaElement ||
    (target instanceof HTMLInputElement &&
      !["checkbox", "radio", "button", "submit", "reset"].includes(target.type))
  )
}

/**
 * Whether a combination belongs to the text field that has the focus.
 * Without ⌘ or Ctrl, `Alt+←` moves by word and a letter is typed: only
 * function keys and the Menu key reach the registry from a field.
 */
function belongsToField(chord: Chord) {
  if (chord.meta || chord.ctrl) return false
  return !/^F[0-9]+$/.test(chord.key) && chord.key !== "ContextMenu"
}

export interface DispatchOptions {
  /** The native macOS bar binds the combinations of `native` itself. */
  nativeMenu: boolean
  native: Set<string>
  bindings: Map<Zone, Array<Binding>>
  context: () => ActionContext
  invoke: (id: string, context: ActionContext) => void
  /** Windows and Linux: `Alt+<mnemonic>` opens a menu of the bar. */
  mnemonic?: (key: string) => boolean
}

/**
 * Handles one key press. Exported for the tests, which drive it with events
 * built for each layout.
 */
export function dispatch(event: KeyboardEvent, options: DispatchOptions) {
  // A key validating a CJK composition is not a shortcut. 229 is what
  // WebKit reports during composition when `isComposing` is not set yet.
  if (event.isComposing || event.keyCode === 229) return
  if (isModifierKey(event.key)) return
  const base = options.context()
  // The zone of the element the key goes to, unless it is a menu of the bar:
  // then the actions see what the focus was on before (context.ts).
  const target = event.target instanceof Element ? event.target : null
  const here = target && !target.closest('[role="menu"], [role="menubar"]')
  const context: ActionContext = here ? { ...base, ...focusOf(target) } : base
  const found = resolve(event, context, options.bindings)
  if (!found) {
    if (
      options.mnemonic &&
      event.altKey &&
      !event.ctrlKey &&
      !event.metaKey &&
      !event.shiftKey
    ) {
      const key = keyOf(event)
      if (key && options.mnemonic(key)) event.preventDefault()
    }
    return
  }
  // Shown only: the zone's component, or the webview, handles it.
  if (found.binding !== "dispatcher") return
  if (isTextField(event.target) && belongsToField(found.chord)) return
  if (options.nativeMenu && options.native.has(found.id)) {
    // The native bar has the one binding: nothing below may handle the key
    // too — CodeMirror's `Mod-/`, the sidebar's `⌘B` — but the default is
    // left to the webview, which hands it to the menu.
    event.stopPropagation()
    return
  }
  const state = availability(found.id, context)
  if (state === "absent") return
  // The combination is the registry's even when the action cannot run now:
  // the webview must not print or reload on a greyed ⌘P or ⌘R.
  event.preventDefault()
  event.stopPropagation()
  if (state === true) options.invoke(found.id, context)
  else console.info(`Ignored ${found.id} from the keyboard: ${state.reason}`)
}

/** Alt is held alone: the menu bar shows its mnemonics (Windows, Linux). */
export const mnemonicsShown = createStore(false)

let mnemonicHandler: ((key: string) => boolean) | undefined

/** The menu bar opens its menus by mnemonic through this. */
export function setMnemonicHandler(handler: ((key: string) => boolean) | null) {
  mnemonicHandler = handler ?? undefined
}

/**
 * Installs the listener. Returns the function that removes it.
 */
export function installKeyboard(): () => void {
  const options: DispatchOptions = {
    nativeMenu,
    native: nativeAccelerators(manifest),
    bindings: bindingsByZone(manifest, platform),
    context: currentContext,
    invoke: (id, context) => void invoke(id, "keyboard", context),
    mnemonic: (key) => mnemonicHandler?.(key) ?? false,
  }
  const onKeyDown = (event: KeyboardEvent) => {
    if (platform === "other")
      mnemonicsShown.setState(() => event.key === "Alt" || event.altKey)
    dispatch(event, options)
  }
  const onKeyUp = (event: KeyboardEvent) => {
    if (event.key === "Alt") mnemonicsShown.setState(() => false)
  }
  const onBlur = () => mnemonicsShown.setState(() => false)
  window.addEventListener("keydown", onKeyDown, true)
  window.addEventListener("keyup", onKeyUp, true)
  window.addEventListener("blur", onBlur)
  return () => {
    window.removeEventListener("keydown", onKeyDown, true)
    window.removeEventListener("keyup", onKeyUp, true)
    window.removeEventListener("blur", onBlur)
  }
}
