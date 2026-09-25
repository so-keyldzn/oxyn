// The static half of the registry, read and checked once
// (docs/adr/0041-registre-d-actions-menus-et-raccourcis.md, point 1). The
// same file is parsed in Rust by `crates/oxyn-desktop/src/menu.rs`: a field
// renamed here is renamed there, in the same commit.

import { z } from "zod"

import manifestJson from "./actions.json"
import { platform } from "./platform"
import { keyCaps, parseChord } from "./shortcut"
import type { Chord, Platform } from "./shortcut"

/**
 * Where a shortcut applies: `app` everywhere, dialogs included; `global`
 * wherever no dialog holds the keyboard; the others where the focus is,
 * named in the DOM by `data-action-zone`.
 */
export const Zone = z.enum([
  "app",
  "global",
  "editor",
  "grid",
  "tabs",
  "tree",
  "assistant",
])
export type Zone = z.infer<typeof Zone>

const PerPlatform = <T extends z.ZodType>(value: T) =>
  z.object({ mac: value.optional(), other: value.optional() })

const Placement = z.object({
  menu: z.string(),
  group: z.number().int().nonnegative(),
  order: z.number().int().nonnegative(),
  /** A letter of the label; Windows and Linux only. */
  mnemonic: z.string().length(1).optional(),
})
export type Placement = z.infer<typeof Placement>

const Shortcuts = z.union([z.string(), z.array(z.string()).min(1)])

const ActionSpec = z.object({
  id: z.string().regex(/^[a-z][a-zA-Z-]*(\.[a-z][a-zA-Z-]*)+$/),
  label: z.union([z.string(), PerPlatform(z.string())]),
  /** Labels that follow the zone with the focus (ADR-0041, point 2). */
  variants: z.array(z.object({ zone: Zone, label: z.string() })).default([]),
  zone: Zone,
  platform: z.enum(["mac", "other"]).optional(),
  binding: z.enum(["dispatcher", "component", "system"]).default("dispatcher"),
  shortcut: PerPlatform(Shortcuts).optional(),
  menu: PerPlatform(Placement).optional(),
  destructive: z.boolean().default(false),
})
export type ActionSpec = z.infer<typeof ActionSpec>

const MenuSpec = z.object({
  id: z.string(),
  title: z.string(),
  platform: z.enum(["mac", "other"]).optional(),
  mnemonic: z.string().length(1).optional(),
  /** macOS system items, placed by `menu.rs`; the web bar has none. */
  roles: z
    .array(
      z.object({
        role: z.string(),
        group: z.number().int().nonnegative(),
        order: z.number().int().nonnegative(),
      })
    )
    .default([]),
})
export type MenuSpec = z.infer<typeof MenuSpec>

export const Manifest = z.object({
  menus: z.array(MenuSpec),
  actions: z.array(ActionSpec),
})
export type Manifest = z.infer<typeof Manifest>

/** The manifest shipped with this build. Throws at import if malformed. */
export const manifest: Manifest = Manifest.parse(manifestJson)

/** Whether the action exists on `on` at all. */
export function onPlatform(spec: Pick<ActionSpec, "platform">, on: Platform) {
  return spec.platform === undefined || spec.platform === on
}

export function labelOf(spec: Pick<ActionSpec, "label">, on: Platform): string {
  if (typeof spec.label === "string") return spec.label
  return spec.label[on] ?? spec.label.other ?? spec.label.mac ?? ""
}

/** The combinations of an action on `on`, first one shown. */
export function chordsOf(
  spec: Pick<ActionSpec, "shortcut">,
  on: Platform
): Array<Chord> {
  const declared = spec.shortcut?.[on]
  if (declared === undefined) return []
  const texts = typeof declared === "string" ? [declared] : declared
  return texts.map((text) => parseChord(text, on))
}

export function shortcutTexts(
  spec: Pick<ActionSpec, "shortcut">,
  on: Platform
): Array<string> {
  const declared = spec.shortcut?.[on]
  if (declared === undefined) return []
  return typeof declared === "string" ? [declared] : declared
}

export function placementOf(
  spec: Pick<ActionSpec, "menu">,
  on: Platform
): Placement | null {
  return spec.menu?.[on] ?? null
}

const byId = new Map(manifest.actions.map((spec) => [spec.id, spec]))

/** The declaration of `id`; the registry test guarantees every id it uses exists. */
export function actionSpec(id: string): ActionSpec | undefined {
  return byId.get(id)
}

/**
 * The key caps of the first combination of `id`, for a `Kbd` beside the
 * button that runs it: written once, in the manifest, never next to the
 * binding it describes.
 */
export function actionKeys(id: string, on: Platform = platform): Array<string> {
  const spec = actionSpec(id)
  const chord = spec ? chordsOf(spec, on)[0] : undefined
  return chord ? keyCaps(chord, on) : []
}

/** The same key caps as one piece of text: `⌘↵` on macOS, `Ctrl+Enter` elsewhere. */
export function actionShortcut(id: string, on: Platform = platform): string {
  return actionKeys(id, on).join(on === "mac" ? "" : "+")
}

/** The `aria-keyshortcuts` value of `id`: `Meta+Enter` on macOS, `Control+Enter` elsewhere. */
export function ariaKeys(
  id: string,
  on: Platform = platform
): string | undefined {
  const spec = actionSpec(id)
  const chord = spec ? chordsOf(spec, on)[0] : undefined
  if (!chord) return undefined
  const key = chord.key.length === 1 ? chord.key.toUpperCase() : chord.key
  return [
    ...(chord.ctrl ? ["Control"] : []),
    ...(chord.alt ? ["Alt"] : []),
    ...(chord.shift ? ["Shift"] : []),
    ...(chord.meta ? ["Meta"] : []),
    key,
  ].join("+")
}
