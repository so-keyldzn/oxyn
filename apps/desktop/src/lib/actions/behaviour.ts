// The shape of a behaviour, and the conditions two modules of behaviours
// share: `registry.ts` and `menu-behaviours.ts` both read them, and neither
// may import the other's values at load time.

import type { ActionContext, ConsoleState } from "./context"

/** `true`, a reason shown with the greyed entry, or not offered at all. */
export type Availability = true | { reason: string } | "absent"

export interface ActionBehaviour {
  enabled: (context: ActionContext) => Availability
  run: (context: ActionContext) => void | Promise<void>
}

/**
 * Whether a console action can run: the one condition the toolbar buttons
 * and every other trigger share (UX-SPEC, « Barre de menus » : « grisés dans
 * les mêmes cas que leurs boutons »).
 */
export function consoleAvailability(
  action: "run" | "cancel" | "save",
  state: ConsoleState
): Availability {
  switch (action) {
    case "run":
      if (state.running) return { reason: "A query is running" }
      if (!state.canRun)
        return { reason: "This console cannot run SQL right now" }
      return true
    case "cancel":
      if (!state.running) return { reason: "Nothing is running" }
      if (state.cancelling) return { reason: "Cancellation is under way" }
      return true
    case "save":
      if (state.writing) return { reason: "A save is under way" }
      return true
  }
}
