// The order the grid draws a result's columns in, when the user moved some
// (UX-SPEC « Souris et glisser »). A display state, like visibility: the
// Arrow indexes, the rows received, the SQL and the export keep the result's
// own order — an export writes what the query returned, not how it was laid
// out, the same rule `Columns` follows for hidden columns (« Colonnes et
// inspection des valeurs »). A copy, which is of what is shown, follows it.

import * as React from "react"

/**
 * `order` with the shown column at position `from` moved to position `to`
 * among the shown ones; hidden columns keep their place in `order`. Returns
 * `order` itself when nothing moves. Pure, so it is tested.
 */
export function moveShownColumn(
  order: ReadonlyArray<number>,
  shown: ReadonlyArray<number>,
  from: number,
  to: number
): ReadonlyArray<number> {
  const moved = shown[from]
  const target = shown[to]
  if (moved === undefined || target === undefined || from === to) return order
  const rest = order.filter((column) => column !== moved)
  const at = rest.indexOf(target)
  if (at === -1) return order
  // Past the target when moving right, before it when moving left.
  rest.splice(to > from ? at + 1 : at, 0, moved)
  return rest
}

/**
 * The order of one result's columns: `null` until the user moves one, and
 * forgotten when another result is shown — a new statement brings other
 * columns under the same indexes.
 */
export function useColumnOrder(resultKey: string, count: number) {
  const [state, setState] = React.useState<{
    result: string
    order: ReadonlyArray<number>
  } | null>(null)
  const order =
    state?.result === resultKey && state.order.length === count
      ? state.order
      : null
  const move = (shown: ReadonlyArray<number>, from: number, to: number) => {
    const current = order ?? Array.from({ length: count }, (_, index) => index)
    const next = moveShownColumn(current, shown, from, to)
    if (next !== current) setState({ result: resultKey, order: next })
  }
  return { order, move }
}
