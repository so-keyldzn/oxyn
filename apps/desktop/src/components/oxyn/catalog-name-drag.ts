// Dragging a relation from the catalog into the SQL editor
// (UX-SPEC « Souris et glisser »): its name is inserted where it is dropped,
// and nothing runs. The name is **quoted by the backend** (`qualifiedName`,
// from `relation_facets`), never joined here: a table called
// `"users"; DROP TABLE audit; --` is legal, and a name glued from its
// segments would put that statement in the console (I-10).

import * as React from "react"

import type { CatalogNode } from "@/lib/ipc/types"
import { trackPointerDrag } from "./pointer-drag"
import type { DragPoint } from "./pointer-drag"

/** What the catalog's parent does with a name taken from it. */
export interface NameTransfer {
  /** A relation was released at `point`: the parent finds what lies there. */
  onDrop: (node: CatalogNode, point: DragPoint) => void
  /** ⌥↵ on a relation — the keyboard's side of the drag: into the console. */
  onInsert: (node: CatalogNode) => void
}

/** Only a relation has a name to insert; a schema or a placeholder does not. */
export function hasInsertableName(node: CatalogNode) {
  return node.address.relation !== null
}

/** The relation being dragged, and where the pointer is. */
export function useNameDrag(transfer: NameTransfer | undefined) {
  const [dragged, setDragged] = React.useState<{
    node: CatalogNode
    point: DragPoint
  } | null>(null)
  const start = (event: React.PointerEvent, node: CatalogNode) => {
    if (!transfer || !hasInsertableName(node)) return
    trackPointerDrag(event, {
      onStart: (point) => setDragged({ node, point }),
      onMove: (point) => setDragged({ node, point }),
      onDrop: (point) => {
        setDragged(null)
        transfer.onDrop(node, point)
      },
      onCancel: () => setDragged(null),
    })
  }
  return { dragged, start }
}
