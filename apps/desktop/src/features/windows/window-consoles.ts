import { windows } from "@/lib/ipc/windows"
import type { WindowConsoles } from "@/lib/ipc/windows"

// Which consoles this window holds, for the workspace file (ADR-0043): the
// next launch reopens them in this window, offline. Every workspace of the
// window — shown or retained behind the start screen — publishes its
// consoles' documents; the window reports them together, at once, and only
// when they changed. Rust keeps nothing a webview names blindly: a document
// another window writes is left out there.

/** What one workspace of the window holds. */
export interface WorkspaceDocuments {
  /** Its consoles' documents, in tab order. */
  documents: Array<string>
  /** The document of the console it shows in front, if any. */
  active: string | null
  /** Shown now: its console in front is the window's. */
  visible: boolean
}

const held = new Map<string, WorkspaceDocuments>()
let sent: string | null = null
let scheduled = false

/** The window's consoles, workspace by workspace in opening order. */
export function windowConsoles(
  workspaces: ReadonlyMap<string, WorkspaceDocuments> = held
): WindowConsoles {
  const documents: Array<string> = []
  let active: string | null = null
  for (const workspace of workspaces.values()) {
    for (const document of workspace.documents)
      if (!documents.includes(document)) documents.push(document)
    if (workspace.visible && workspace.active !== null)
      active = workspace.active
  }
  return { documents, active }
}

// Coalesced: a render that changes several workspaces writes once.
function schedule() {
  if (scheduled) return
  scheduled = true
  queueMicrotask(() => {
    scheduled = false
    const next = windowConsoles()
    const key = JSON.stringify(next)
    if (key === sent) return
    sent = key
    // A failed write costs the layout of the next launch, never a console:
    // the next change writes the whole list again.
    void windows.reportConsoles(next).catch(() => {
      sent = null
    })
  })
}

/** Called by each workspace whenever its consoles or the one in front change. */
export function publishWorkspaceDocuments(
  workspace: string,
  documents: WorkspaceDocuments
) {
  held.set(workspace, documents)
  schedule()
}

/** The workspace is gone: its consoles leave the window. */
export function withdrawWorkspaceDocuments(workspace: string) {
  if (held.delete(workspace)) schedule()
}
