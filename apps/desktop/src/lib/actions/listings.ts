  // What the palette and the shortcut sheet list
// (docs/adr/0041-registre-d-actions-menus-et-raccourcis.md, point 7). Both
// read the manifest and the registry: neither keeps a list of its own, so an
// action added to the registry appears in them, with the label, combination
// and greyed state of every other trigger.

import type { ActionContext } from "./context"
import { chordsOf, labelOf, onPlatform, placementOf } from "./manifest"
import type { ActionSpec, Manifest, Zone } from "./manifest"
import { masked, variantOf } from "./menu-model"
import { availability as liveAvailability, checkedOf } from "./registry"
import type { Availability } from "./registry"
import { keyCaps } from "./shortcut"
import type { Platform } from "./shortcut"

/** The palette does not list itself: choosing it from it would do nothing. */
const PALETTE = "palette.open"

/** The section of an action placed in no menu. */
const UNPLACED = "More"

export interface PaletteEntry {
  id: string
  /** `Text size: Compact` for an entry of a submenu. */
  label: string
  /** The title of the menu the action sits in. */
  section: string
  /** The key caps of its combination, empty when it has none here. */
  keys: Array<string>
  availability: true | { reason: string }
  checked: boolean | null
}

export interface PaletteSection {
  title: string
  entries: Array<PaletteEntry>
}

/** Where an action sits: its top menu, the submenu title, and the order. */
function locate(source: Manifest, spec: ActionSpec, on: Platform) {
  const menus = source.menus.filter((menu) => onPlatform(menu, on))
  const placement = placementOf(spec, on)
  const menu = menus.find((candidate) => candidate.id === placement?.menu)
  if (!placement || !menu) return null
  const parent = menu.parent?.[on]
  const top = parent
    ? menus.find((candidate) => candidate.id === parent.menu)
    : menu
  if (!top) return null
  return {
    top,
    submenu: parent ? menu.title : null,
    rank: [
      menus.indexOf(top),
      parent?.group ?? placement.group,
      parent?.order ?? placement.order,
      parent ? placement.group : 0,
      parent ? placement.order : 0,
    ],
  }
}

function compareRanks(left: Array<number>, right: Array<number>) {
  for (const [index, value] of left.entries()) {
    const other = right[index] ?? 0
    if (value !== other) return value - other
  }
  return 0
}

/**
 * The palette in `context`: every action of the registry offered here, by
 * menu, the greyed ones with their reason — on macOS the only place where the
 * reason of a greyed entry of the native bar can be read (ADR-0041,
 * « Conséquences »). An absent action has no entry, as in the menus.
 */
export function paletteSections(
  source: Manifest,
  on: Platform,
  context: ActionContext,
  availabilityOf: (
    id: string,
    context: ActionContext
  ) => Availability = liveAvailability,
  checkedFor: (id: string, context: ActionContext) => boolean | null = checkedOf
): Array<PaletteSection> {
  const listed = source.actions.flatMap((spec) => {
    if (spec.id === PALETTE || !onPlatform(spec, on)) return []
    const state = availabilityOf(spec.id, context)
    if (state === "absent") return []
    const place = locate(source, spec, on)
    const variant = variantOf(spec, context.zones)
    const label =
      variant === 0
        ? labelOf(spec, on)
        : (spec.variants[variant - 1]?.label ?? labelOf(spec, on))
    const chord = chordsOf(spec, on)[0]
    const entry: PaletteEntry = {
      id: spec.id,
      label: place?.submenu ? `${place.submenu}: ${label}` : label,
      section: place?.top.title ?? UNPLACED,
      keys:
        chord && !masked(spec, source, on, context.zones)
          ? keyCaps(chord, on)
          : [],
      availability: state,
      checked: checkedFor(spec.id, context),
    }
    return [{ entry, rank: place?.rank ?? [Number.MAX_SAFE_INTEGER] }]
  })
  listed.sort((left, right) => compareRanks(left.rank, right.rank))
  const sections: Array<PaletteSection> = []
  for (const { entry } of listed) {
    const last = sections.at(-1)
    if (last?.title === entry.section) last.entries.push(entry)
    else sections.push({ title: entry.section, entries: [entry] })
  }
  return sections
}

export interface SheetRow {
  id: string
  label: string
  /** Every combination of the action, each as its key caps. */
  keys: Array<Array<string>>
}

export interface SheetSection {
  title: string
  rows: Array<SheetRow>
}

/** The zones as the sheet names them, in the order it shows them. */
const ZONE_TITLES: Array<[Array<Zone>, string]> = [
  [["app", "global"], "Everywhere"],
  [["editor"], "SQL editor"],
  [["grid"], "Results"],
  [["tree"], "Catalog"],
  [["tabs"], "Tabs"],
  [["assistant"], "Assistant"],
]

/**
 * The shortcut sheet of `on`: every combination of the manifest, by zone —
 * those a component handles itself (`Esc` in the console) included. It does
 * not depend on the context: a shortcut exists whether or not it can run.
 */
export function shortcutSheet(
  source: Manifest,
  on: Platform
): Array<SheetSection> {
  return ZONE_TITLES.flatMap(([zones, title]) => {
    const rows = source.actions.flatMap((spec) => {
      if (!onPlatform(spec, on) || !zones.includes(spec.zone)) return []
      const keys = chordsOf(spec, on).map((chord) => keyCaps(chord, on))
      if (keys.length === 0) return []
      return [{ id: spec.id, label: labelOf(spec, on), keys }]
    })
    return rows.length > 0 ? [{ title, rows }] : []
  })
}
