/**
 * What the webview would do as a browser, cancelled: the page's context menu,
 * reload, page zoom, back and forward (ADR-0041 § 8; UX-SPEC, « Ce qu'Oxyn ne
 * fait pas, parce que ce n'est pas un navigateur »).
 *
 * Every listener only calls `preventDefault`, never `stopPropagation`: the
 * browser's default is what goes, and a component that binds the same keys —
 * the diagram's `+`, a future Text size entry — still receives them. The
 * `on_navigation` guard of `crates/oxyn-desktop/src/webview_guard.rs` is the
 * last line behind this one.
 */

/** Keys that reload the page, on any platform: a reload loses the window's
 * unsaved state and orphans the tasks feeding its `Channel`s. */
function reloads(event: KeyboardEvent): boolean {
  if (event.key === "F5" || event.key === "BrowserRefresh") return true
  // `event.code`: `R` is not `r` under ⌥, nor on a Cyrillic layout.
  return (event.metaKey || event.ctrlKey) && event.code === "KeyR"
}

/** The physical keys of page zoom — by `event.code`, since AZERTY needs ⇧
 * for its digits and puts `=` elsewhere. */
const ZOOM_CODES = new Set([
  "Equal",
  "Minus",
  "Digit0",
  "NumpadAdd",
  "NumpadSubtract",
  "Numpad0",
])

function zooms(event: KeyboardEvent): boolean {
  return (event.metaKey || event.ctrlKey) && ZOOM_CODES.has(event.code)
}

/** History keys. `Alt+←` and `Alt+→` are WebView2's back and forward; under
 * macOS they move the caret by a word, and WebKit binds no history to them. */
function navigatesHistory(event: KeyboardEvent, mac: boolean): boolean {
  if (event.key === "BrowserBack" || event.key === "BrowserForward") return true
  return (
    !mac &&
    event.altKey &&
    !event.ctrlKey &&
    !event.metaKey &&
    (event.key === "ArrowLeft" || event.key === "ArrowRight")
  )
}

/** Input types that hold text a user edits, and so keep Cut, Copy, Paste. */
const TEXT_INPUT_TYPES = new Set([
  "",
  "text",
  "search",
  "url",
  "email",
  "tel",
  "password",
  "number",
])

/** Whether `target` is inside a text field that has no menu of its own —
 * the only place the webview's menu stays, for its Cut, Copy and Paste. */
export function inTextField(target: EventTarget | null): boolean {
  if (!(target instanceof Element)) return false
  const field = target.closest(
    'input, textarea, [contenteditable]:not([contenteditable="false"])'
  )
  if (field === null) return false
  if (field instanceof HTMLInputElement)
    return TEXT_INPUT_TYPES.has(field.getAttribute("type") ?? "")
  return true
}

/** The mouse's back and forward buttons. */
const BACK_BUTTON = 3
const FORWARD_BUTTON = 4

/** A region that owns its zoom: the `erd` diagram zooms itself, pinch
 * included, and touches nothing else (UX-SPEC). */
const OWN_ZOOM = "[data-own-zoom]"

function ownsZoom(target: EventTarget | null): boolean {
  return target instanceof Element && target.closest(OWN_ZOOM) !== null
}

export function isMacPlatform(view: Window): boolean {
  return /Mac|iPhone|iPad/.test(view.navigator.userAgent)
}

/**
 * Installs the listeners on `view`, once for the application. Returns what
 * removes them.
 */
export function suppressBrowserDefaults(
  view: Window,
  mac: boolean = isMacPlatform(view)
): () => void {
  const onContextMenu = (event: MouseEvent) => {
    // A menu of Oxyn's (catalog tree, and the surfaces of ADR-0041 § 6) has
    // already taken the event.
    if (event.defaultPrevented) return
    if (inTextField(event.target)) return
    event.preventDefault()
  }
  const onKeyDown = (event: KeyboardEvent) => {
    if (event.isComposing) return
    if (reloads(event) || zooms(event) || navigatesHistory(event, mac))
      event.preventDefault()
  }
  const onMouseButton = (event: MouseEvent) => {
    if (event.button === BACK_BUTTON || event.button === FORWARD_BUTTON)
      event.preventDefault()
  }
  // A trackpad pinch reaches Chromium and WebKit as a wheel with `ctrlKey`.
  const onWheel = (event: WheelEvent) => {
    if (event.ctrlKey && !ownsZoom(event.target)) event.preventDefault()
  }
  // WebKit's own pinch events (`gesturestart`, `gesturechange`), which no
  // DOM typing names.
  const onGesture = (event: Event) => {
    if (!ownsZoom(event.target)) event.preventDefault()
  }

  const document = view.document
  document.addEventListener("contextmenu", onContextMenu)
  view.addEventListener("keydown", onKeyDown, { capture: true })
  view.addEventListener("mouseup", onMouseButton, { capture: true })
  view.addEventListener("auxclick", onMouseButton, { capture: true })
  view.addEventListener("wheel", onWheel, { capture: true, passive: false })
  view.addEventListener("gesturestart", onGesture, { capture: true })
  view.addEventListener("gesturechange", onGesture, { capture: true })
  return () => {
    document.removeEventListener("contextmenu", onContextMenu)
    view.removeEventListener("keydown", onKeyDown, { capture: true })
    view.removeEventListener("mouseup", onMouseButton, { capture: true })
    view.removeEventListener("auxclick", onMouseButton, { capture: true })
    view.removeEventListener("wheel", onWheel, { capture: true })
    view.removeEventListener("gesturestart", onGesture, { capture: true })
    view.removeEventListener("gesturechange", onGesture, { capture: true })
  }
}
