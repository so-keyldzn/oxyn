// The menu bar as the manifest describes it, and the state of each entry in
// a context (docs/adr/0041-registre-d-actions-menus-et-raccourcis.md, points
// 2, 4 and 5). The web bar of Windows and Linux draws the model; the native
// bar of macOS is built by Rust from the same file and receives the states.

import type { ActionContext } from "./context"
import { chordsOf, labelOf, onPlatform, placementOf } from "./manifest"
import type { ActionSpec, Manifest, Zone } from "./manifest"
import { availability as liveAvailability } from "./registry"
import type { Availability } from "./registry"
import { sameChord } from "./shortcut"
import type { Chord, Platform } from "./shortcut"

export interface MenuEntry {
  id: string
  /** The default label, then one per zone variant, in manifest order. */
  labels: Array<string>
  mnemonic: string | null
  chord: Chord | null
}

export interface MenuModel {
  id: string
  title: string
  mnemonic: string | null
  /** Entries by group; a separator is drawn between groups. */
  groups: Array<Array<MenuEntry>>
}

/** The menus of `on`, in manifest order, without the empty ones. */
export function menuBar(source: Manifest, on: Platform): Array<MenuModel> {
  return source.menus
    .filter((menu) => menu.platform === undefined || menu.platform === on)
    .map((menu) => {
      const placed = source.actions
        .filter((spec) => onPlatform(spec, on))
        .flatMap((spec) => {
          const placement = placementOf(spec, on)
          return placement?.menu === menu.id ? [{ spec, placement }] : []
        })
        .sort(
          (left, right) =>
            left.placement.group - right.placement.group ||
            left.placement.order - right.placement.order
        )
      const groups: Array<Array<MenuEntry>> = []
      let group: number | null = null
      for (const { spec, placement } of placed) {
        if (placement.group !== group) {
          groups.push([])
          group = placement.group
        }
        groups[groups.length - 1]?.push({
          id: spec.id,
          labels: [
            labelOf(spec, on),
            ...spec.variants.map((variant) => variant.label),
          ],
          mnemonic: placement.mnemonic ?? null,
          chord: chordsOf(spec, on)[0] ?? null,
        })
      }
      return {
        id: menu.id,
        title: menu.title,
        mnemonic: menu.mnemonic ?? null,
        groups,
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
}

/** The variant of `spec` for the zones around the focus, innermost first. */
function variantOf(spec: ActionSpec, zones: Array<Zone>) {
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
function masked(
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
  ) => Availability = liveAvailability
): Array<EntryState> {
  return source.actions
    .filter((spec) => onPlatform(spec, on) && placementOf(spec, on) !== null)
    .map((spec) => ({
      id: spec.id,
      availability: availabilityOf(spec.id, context),
      variant: variantOf(spec, context.zones),
      shortcut: !masked(spec, source, on, context.zones),
    }))
}
