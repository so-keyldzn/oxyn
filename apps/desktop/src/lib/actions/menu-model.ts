// The menu bar as the manifest describes it, and the state of each entry in
// a context (docs/adr/0041-registre-d-actions-menus-et-raccourcis.md, points
// 2, 4 and 5). The web bar of Windows and Linux draws the model; the native
// bar of macOS is built by Rust from the same file and receives the states.

import type { ActionContext } from "./context"
import { chordsOf, labelOf, onPlatform, placementOf } from "./manifest"
import type { ActionSpec, Manifest, Zone } from "./manifest"
import {
  checkedOf as liveChecked,
  availability as liveAvailability,
} from "./registry"
import type { Availability } from "./registry"
import { sameChord } from "./shortcut"
import type { Chord, Platform } from "./shortcut"

export interface MenuEntry {
  kind: "action"
  id: string
  /** The default label, then one per zone variant, in manifest order. */
  labels: Array<string>
  mnemonic: string | null
  chord: Chord | null
  /** Drawn with a check mark (`View ▸ Theme ▸ Dark`). */
  check: boolean
}

/** A submenu of a menu (`View ▸ Text size`), one level deep. */
export interface SubmenuEntry {
  kind: "submenu"
  id: string
  title: string
  mnemonic: string | null
  groups: Array<Array<MenuEntry>>
}

export type MenuItemModel = MenuEntry | SubmenuEntry

export interface MenuModel {
  id: string
  title: string
  mnemonic: string | null
  /** Entries by group; a separator is drawn between groups. */
  groups: Array<Array<MenuItemModel>>
}

interface Placed<T> {
  item: T
  group: number
  order: number
}

/** Sorts by group then order, and cuts where the group changes. */
function grouped<T>(placed: Array<Placed<T>>): Array<Array<T>> {
  const groups: Array<Array<T>> = []
  let group: number | null = null
  for (const entry of [...placed].sort(
    (left, right) => left.group - right.group || left.order - right.order
  )) {
    if (entry.group !== group) {
      groups.push([])
      group = entry.group
    }
    groups[groups.length - 1]?.push(entry.item)
  }
  return groups
}

/** The actions of `on` placed in `menu`, as entries. */
function actionsIn(
  source: Manifest,
  on: Platform,
  menu: string
): Array<Placed<MenuEntry>> {
  return source.actions
    .filter((spec) => onPlatform(spec, on))
    .flatMap((spec) => {
      const placement = placementOf(spec, on)
      if (placement?.menu !== menu) return []
      return [
        {
          item: {
            kind: "action" as const,
            id: spec.id,
            labels: [
              labelOf(spec, on),
              ...spec.variants.map((variant) => variant.label),
            ],
            mnemonic: placement.mnemonic ?? null,
            chord: chordsOf(spec, on)[0] ?? null,
            check: spec.check,
          },
          group: placement.group,
          order: placement.order,
        },
      ]
    })
}

/** The menus of `on`, in manifest order, without the empty ones. */
export function menuBar(source: Manifest, on: Platform): Array<MenuModel> {
  const menus = source.menus.filter(
    (menu) => menu.platform === undefined || menu.platform === on
  )
  return menus
    .filter((menu) => menu.parent === undefined)
    .map((menu) => {
      const submenus = menus.flatMap((sub): Array<Placed<MenuItemModel>> => {
        const placement = sub.parent?.[on]
        if (placement?.menu !== menu.id) return []
        const groups = grouped(actionsIn(source, on, sub.id))
        if (groups.length === 0) return []
        return [
          {
            item: {
              kind: "submenu",
              id: sub.id,
              title: sub.title,
              mnemonic: placement.mnemonic ?? null,
              groups,
            },
            group: placement.group,
            order: placement.order,
          },
        ]
      })
      return {
        id: menu.id,
        title: menu.title,
        mnemonic: menu.mnemonic ?? null,
        groups: grouped<MenuItemModel>([
          ...actionsIn(source, on, menu.id),
          ...submenus,
        ]),
      }
    })
    .filter((menu) => menu.groups.length > 0)
}

export interface EntryState {
  id: string
  availability: Availability
  /** 0 for the default label, `n` for the `n`-th variant. */
  variant: number
  /** The combination is shown: no zone with the focus takes it. */
  shortcut: boolean
  /** The check mark of a `check` action; `null` for the others. */
  checked: boolean | null
}

/** The variant of `spec` for the zones around the focus, innermost first. */
export function variantOf(spec: ActionSpec, zones: Array<Zone>) {
  for (const zone of zones) {
    const index = spec.variants.findIndex((variant) => variant.zone === zone)
    if (index !== -1) return index + 1
  }
  return 0
}

/**
 * Whether a zone around the focus binds the same combination as `spec`.
 * The menu then shows the entry without it (ADR-0041, point 2).
 */
export function masked(
  spec: ActionSpec,
  source: Manifest,
  on: Platform,
  zones: Array<Zone>
) {
  if (spec.zone !== "global" && spec.zone !== "app") return false
  const own = chordsOf(spec, on)[0]
  if (!own) return false
  return source.actions.some(
    (other) =>
      other.id !== spec.id &&
      onPlatform(other, on) &&
      zones.includes(other.zone) &&
      chordsOf(other, on).some((chord) => sameChord(chord, own))
  )
}

/** The state of every entry of the bar of `on`, in `context`. */
export function entryStates(
  source: Manifest,
  on: Platform,
  context: ActionContext,
  availabilityOf: (
    id: string,
    context: ActionContext
  ) => Availability = liveAvailability,
  checkedFor: (
    id: string,
    context: ActionContext
  ) => boolean | null = liveChecked
): Array<EntryState> {
  return source.actions
    .filter((spec) => onPlatform(spec, on) && placementOf(spec, on) !== null)
    .map((spec) => ({
      id: spec.id,
      availability: availabilityOf(spec.id, context),
      variant: variantOf(spec, context.zones),
      shortcut: !masked(spec, source, on, context.zones),
      checked: checkedFor(spec.id, context),
    }))
}
