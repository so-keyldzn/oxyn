import { closeConversation } from "@/features/assistant/conversation-store"
import { previews } from "@/features/metadata/use-preview"
import type { WorkspaceExit } from "@/features/workspace/workspace-screen"
import { backend } from "@/lib/ipc/client"
import { consoles } from "@/lib/ipc/consoles"
import type { OpenConnection } from "@/lib/ipc/types"

/**
 * Closes what a workspace left: every session, or the whole connection.
 *
 * `reopened` is asked once the drafts are written, not before: a connection
 * reopened meanwhile has new sessions that `disconnect` would close.
 */
export async function release(
  previous: OpenConnection,
  reopened: () => boolean,
  exit: WorkspaceExit | null
) {
  const left = exit ? await exit() : { sessions: [previous.session] }
  // Read on sessions about to close: never shown again as current, and a
  // Refresh must not reach a closed session.
  previews.closeSessions([previous.session, ...left.sessions])
  if (reopened()) {
    // Reopened on the same connection: `disconnect` would close the new
    // sessions too, so only the old ones go.
    await Promise.allSettled(
      left.sessions.map((id) => consoles.close(previous.connection, id))
    )
    return
  }
  await backend.disconnect(previous.connection).catch(() => undefined)
  // The connection is closed: its conversations go with it, and so does any
  // external agent still running for them — an agent kept alive past its
  // connection would keep the tools and the tier it was launched under (I-04).
  await closeConversation(previous.connection)
}
