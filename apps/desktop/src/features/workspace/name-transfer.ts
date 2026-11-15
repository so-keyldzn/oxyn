import type { NameTransfer } from "@/components/oxyn/catalog-name-drag"
import { consoleDestination } from "@/features/consoles/text-destination"
import { openInConsole } from "@/features/consoles/open-in-console"
import { zoneHandleAt } from "@/lib/actions/context"
import { metadata } from "@/lib/ipc/metadata"
import type { CatalogNode } from "@/lib/ipc/types"

/**
 * Where a relation's name goes when taken from the catalog: into the editor
 * it was dropped on, or — ⌥↵ — into the active console.
 *
 * The name is the backend's `qualifiedName`, quoted by the dialect from the
 * catalog cache; nothing here joins or quotes a segment (I-10). Nothing runs:
 * the text lands in the editor, and `Run` stays the user's.
 */
export function nameTransfer(
  connection: string,
  onProblem: (caught: unknown) => void,
  source?: { session: string; private: boolean },
  onCrossDrop?: (node: CatalogNode) => void
): NameTransfer {
  const quotedName = async (node: CatalogNode) =>
    (
      await (source
        ? metadata.relationFacets(connection, node.address, source.session)
        : metadata.relationFacets(connection, node.address))
    ).qualifiedName
  return {
    onDrop: (node, point) => {
      // Read where it fell before the name is asked for: by the time it
      // arrives, the pointer — and what lies under it — has moved on.
      const editor = zoneHandleAt("editor", point)
      if (!editor?.insertAt) return
      if (
        editor.owner?.connection !== connection ||
        (source?.private && editor.owner.session !== source.session)
      ) {
        if (onCrossDrop) onCrossDrop(node)
        else
          onProblem(
            new Error(
              "The destination editor belongs to another connection or private session. Nothing was inserted."
            )
          )
        return
      }
      const insert = editor.insertAt
      quotedName(node).then((name) => insert(name, point), onProblem)
    },
    onInsert: (node) => {
      const destination = consoleDestination(connection)
      if (!destination) {
        onProblem(new Error("The source workspace is no longer open."))
        return
      }
      quotedName(node).then(
        (name) =>
          openInConsole(connection, name, {
            workspace: destination.workspace,
            session: source?.private ? source.session : destination.session,
            notice: `The name of ${node.name} was placed here from the catalog. Nothing was executed.`,
          }),
        onProblem
      )
    },
  }
}
