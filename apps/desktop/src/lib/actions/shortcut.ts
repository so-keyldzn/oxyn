// Combinations as the manifest writes them (`Mod+Shift+Enter`), and the
// matching rules of ADR-0041, point 3. Pure functions: the platform is always
// an argument, so a test reads a macOS and a Windows keyboard side by side.

export type Platform = "mac" | "other"

/** A combination resolved for one platform: `Mod` is gone. */
export interface Chord {
  meta: boolean
  ctrl: boolean
  alt: boolean
  shift: boolean
  /** A lowercase character (`t`, `1`, `/`) or a key name (`Enter`, `F10`). */
  key: string
}

const MODIFIERS = new Set(["Mod", "Ctrl", "Alt", "Shift", "Meta"])

/** Named keys a combination may use; everything else is one character. */
const NAMED =
  /^(Enter|Escape|Tab|Space|Backspace|Delete|Home|End|PageUp|PageDown|ArrowUp|ArrowDown|ArrowLeft|ArrowRight|ContextMenu|F([1-9]|1[0-9]|2[0-4]))$/

/**
 * Parses `Mod+Shift+Enter` for `platform`, or throws: an unreadable manifest
 * must fail its test, not drop a shortcut at run time.
 */
export function parseChord(text: string, platform: Platform): Chord {
  const parts = text.split("+")
  // `Mod++` would name the `+` key: split leaves two empty strings.
  const key = parts.pop()
  if (key === undefined || key === "") throw new Error(`"${text}" names no key`)
  const chord: Chord = {
    meta: false,
    ctrl: false,
    alt: false,
    shift: false,
    key: key.length === 1 ? key.toLowerCase() : key,
  }
  for (const part of parts) {
    if (!MODIFIERS.has(part))
      throw new Error(`"${text}": "${part}" is not a modifier`)
    if (part === "Mod") {
      if (platform === "mac") chord.meta = true
      else chord.ctrl = true
    } else if (part === "Ctrl") chord.ctrl = true
    else if (part === "Alt") chord.alt = true
    else if (part === "Shift") chord.shift = true
    else chord.meta = true
  }
  if (key.length !== 1 && !NAMED.test(key))
    throw new Error(`"${text}": "${key}" is not a key this manifest knows`)
  if (key.length === 1 && !isPrintable(key))
    throw new Error(`"${text}": "${key}" is not a printable ASCII character`)
  return chord
}

function isPrintable(character: string) {
  return character.length === 1 && character >= "!" && character <= "~"
}

/** A single printable character that is neither a letter nor a digit. */
export function isPunctuation(key: string) {
  return key.length === 1 && isPrintable(key) && !/[a-z0-9]/i.test(key)
}

/**
 * The identity of a pressed key.
 *
 * Digits are always read from `event.code`: on AZERTY they need ⇧, and
 * `event.key` is `&` for ⌘1. Otherwise `event.key` when it is one printable
 * ASCII character — what the layout produces, so `/` is `/` on AZERTY too —
 * and else the letter of `event.code`: ⌥ on macOS turns `b` into `∫`, and a
 * Cyrillic layout produces `и` where the key cap says B.
 */
export function keyOf(
  event: Pick<KeyboardEvent, "key" | "code">
): string | null {
  const digit = /^Digit([0-9])$/.exec(event.code)
  if (digit) return digit[1] ?? null
  if (event.key === " ") return "Space"
  if (event.key.length === 1 && isPrintable(event.key))
    return event.key.toLowerCase()
  const letter = /^Key([A-Z])$/.exec(event.code)
  if (letter) return letter[1]?.toLowerCase() ?? null
  if (event.key === "Dead" || event.key === "Unidentified") return null
  return event.key
}

/** The modifier keys themselves never make a combination. */
export function isModifierKey(key: string) {
  return (
    key === "Shift" ||
    key === "Control" ||
    key === "Alt" ||
    key === "Meta" ||
    key === "AltGraph" ||
    key === "OS"
  )
}

/**
 * Whether `event` presses `chord`.
 *
 * ⇧ is not compared for a punctuation key, since it may be what produces it
 * (`/` is ⇧: on AZERTY) — which is why the manifest refuses
 * `Mod+Shift+<punctuation>`.
 */
export function matches(
  chord: Chord,
  event: Pick<
    KeyboardEvent,
    "key" | "code" | "metaKey" | "ctrlKey" | "altKey" | "shiftKey"
  >
): boolean {
  if (keyOf(event) !== chord.key) return false
  if (event.metaKey !== chord.meta || event.ctrlKey !== chord.ctrl) return false
  if (event.altKey !== chord.alt) return false
  return isPunctuation(chord.key) || event.shiftKey === chord.shift
}

/** Whether `event` holds the platform's command key, and only that one. */
export function hasMod(
  event: Pick<KeyboardEvent, "metaKey" | "ctrlKey">,
  platform: Platform
) {
  return platform === "mac"
    ? event.metaKey && !event.ctrlKey
    : event.ctrlKey && !event.metaKey
}

/**
 * Why a combination may not be declared, or `null`.
 *
 * - `Mod+Shift+<punctuation>`: ⇧ is not compared for punctuation, so the
 *   action would also fire without it, and on AZERTY it may be required to
 *   type the character at all.
 * - `Ctrl+Alt+<printable>` outside macOS: AltGr is reported as Ctrl+Alt, and
 *   the combination would steal a character (`AltGr+0` is `@` on French
 *   AZERTY).
 */
export function forbiddenReason(
  text: string,
  platform: Platform
): string | null {
  const chord = parseChord(text, platform)
  const command = platform === "mac" ? chord.meta : chord.ctrl
  if (command && chord.shift && isPunctuation(chord.key))
    return `${text}: ⇧ is not compared for punctuation`
  if (platform === "other" && chord.ctrl && chord.alt && chord.key.length === 1)
    return `${text}: Ctrl+Alt is AltGr outside macOS, and steals a character`
  return null
}

const MAC_KEYS: Record<string, string> = {
  Enter: "↵",
  Escape: "Esc",
  Backspace: "⌫",
  Delete: "⌦",
  ArrowUp: "↑",
  ArrowDown: "↓",
  ArrowLeft: "←",
  ArrowRight: "→",
  ContextMenu: "Menu",
}

const OTHER_KEYS: Record<string, string> = {
  Escape: "Esc",
  ArrowUp: "Up",
  ArrowDown: "Down",
  ArrowLeft: "Left",
  ArrowRight: "Right",
  ContextMenu: "Menu",
}

/**
 * The key caps of a combination, as `Kbd` shows them: `⌘ ⇧ ↵` on macOS,
 * `Ctrl Shift Enter` elsewhere.
 *
 * A punctuation key is shown by its character (`⌘/`), not by the key to press
 * on the active layout: that needs a layout map not every webview has
 * (ADR-0041, point to verify no. 7).
 */
export function keyCaps(chord: Chord, platform: Platform): Array<string> {
  const key =
    chord.key.length === 1
      ? chord.key.toUpperCase()
      : ((platform === "mac" ? MAC_KEYS : OTHER_KEYS)[chord.key] ?? chord.key)
  if (platform === "mac") {
    // Apple's order: ⌃ ⌥ ⇧ ⌘.
    return [
      ...(chord.ctrl ? ["⌃"] : []),
      ...(chord.alt ? ["⌥"] : []),
      ...(chord.shift ? ["⇧"] : []),
      ...(chord.meta ? ["⌘"] : []),
      key,
    ]
  }
  return [
    ...(chord.ctrl ? ["Ctrl"] : []),
    ...(chord.alt ? ["Alt"] : []),
    ...(chord.shift ? ["Shift"] : []),
    ...(chord.meta ? ["Win"] : []),
    key,
  ]
}

/** Two combinations a single key press would both match. */
export function sameChord(left: Chord, right: Chord) {
  return (
    left.key === right.key &&
    left.meta === right.meta &&
    left.ctrl === right.ctrl &&
    left.alt === right.alt &&
    (isPunctuation(left.key) || left.shift === right.shift)
  )
}
