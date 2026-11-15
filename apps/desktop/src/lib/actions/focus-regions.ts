/** Optional focus regions follow the rendered workspace, including future maps. */
const REGION = '[data-focus-region], [data-slot="sidebar"]'
const lastFocus = new WeakMap<HTMLElement, HTMLElement>()
const focusEntries = new WeakMap<HTMLElement, () => void>()
const FOCUSABLE =
  'button, input, textarea, select, a[href], [tabindex], [contenteditable="true"]'

function concealed(element: HTMLElement) {
  for (
    let parent: HTMLElement | null = element;
    parent;
    parent = parent.parentElement
  ) {
    if (
      parent.matches(
        '[hidden], [inert], [data-hidden], [aria-hidden="true"], [data-focus-disabled="true"], [data-slot="sidebar"][data-state="collapsed"]'
      )
    )
      return true
    const style = getComputedStyle(parent)
    if (style.display === "none" || style.visibility === "hidden") return true
  }
  return false
}

function shown(element: HTMLElement) {
  return (
    element.isConnected &&
    element.getClientRects().length > 0 &&
    !concealed(element)
  )
}

function usable(element: HTMLElement) {
  return (
    shown(element) &&
    !element.matches(
      ':disabled, [aria-disabled="true"], [role="tab"][aria-selected="false"]'
    )
  )
}

/** The enclosing primary region; nested editors belong to their side panel. */
function regionOf(element: Element | null): HTMLElement | null {
  let region = element?.closest<HTMLElement>(REGION) ?? null
  while (region?.parentElement?.closest(REGION))
    region = region.parentElement.closest<HTMLElement>(REGION)
  return region
}

/** Called by the existing focus tracker, rather than another global listener. */
export function rememberRegionFocus(element: Element | null) {
  const region = regionOf(element)
  if (region && element instanceof HTMLElement) lastFocus.set(region, element)
}

/** Editor engines restore their own selection when their content receives focus. */
export function setFocusEntry(
  element: HTMLElement,
  focus: (() => void) | null
) {
  if (focus) focusEntries.set(element, focus)
  else focusEntries.delete(element)
}

function targetOf(region: HTMLElement) {
  const previous = lastFocus.get(region)
  if (previous && region.contains(previous) && usable(previous)) return previous
  const candidates = [
    ...region.querySelectorAll<HTMLElement>(FOCUSABLE),
  ].filter(
    (element) =>
      usable(element) && (element.tabIndex >= 0 || element.isContentEditable)
  )
  return (
    candidates.find((element) =>
      element.matches(
        '[data-focus-entry], .cm-content, [role="grid"], [role="tree"]'
      )
    ) ??
    candidates[0] ??
    (region.hasAttribute("tabindex") && usable(region) ? region : null)
  )
}

/** No panels are opened, no source activated, and no data requested by F6. */
export function cycleFocus(
  focused: Element | null,
  direction: -1 | 1
): boolean {
  if (typeof document === "undefined") return false
  if (
    [
      ...document.querySelectorAll<HTMLElement>(
        '[role="dialog"], [role="alertdialog"]'
      ),
    ].some(shown)
  )
    return false
  const focusedRoot = focused?.closest<HTMLElement>("[data-focus-workspace]")
  const root =
    focusedRoot && !concealed(focusedRoot)
      ? focusedRoot
      : [
          ...document.querySelectorAll<HTMLElement>("[data-focus-workspace]"),
        ].find((element) => !concealed(element))
  if (!root) return false
  rememberRegionFocus(focused)
  const entries = [...root.querySelectorAll<HTMLElement>(REGION)]
    .filter((region) => !region.parentElement?.closest(REGION))
    .map((region) => ({ region, target: targetOf(region) }))
    .filter(
      (entry): entry is { region: HTMLElement; target: HTMLElement } =>
        entry.target !== null
    )
  if (entries.length === 0) return false
  const current = entries.findIndex(
    (entry) => entry.region === regionOf(focused)
  )
  const index =
    current < 0
      ? direction === 1
        ? 0
        : entries.length - 1
      : (current + direction + entries.length) % entries.length
  const target = entries[index]?.target
  if (!target) return false
  const restore = focusEntries.get(target)
  if (restore) restore()
  else target.focus()
  return true
}
