import * as React from "react"
import { GroupDisconnectDialog } from "@/components/oxyn/group-disconnect-dialog"
import type { GroupDisconnectMember } from "@/components/oxyn/group-disconnect-dialog"
import { session, showConnection } from "@/features/session"
import {
  readDisconnectCosts,
  reviewWorkspaceDisconnect,
} from "@/features/workspace/disconnect-requests"
import type { ServerProfileSnapshot } from "@/lib/ipc/server-profiles"

/** Serial owner reviews stop at the first failure; completed closes stay explicit. */
export function useGroupDisconnect(
  transactions: Record<string, string>,
  report: (caught: unknown) => void
) {
  const [review, setReview] = React.useState<{
    name: string
    members: Array<GroupDisconnectMember>
  } | null>(null)
  const [busy, setBusy] = React.useState(false)
  const [result, setResult] = React.useState<string | null>(null)
  const running = React.useRef(false)
  const request = (group: ServerProfileSnapshot) => {
    if (running.current) return
    setReview({
      name: group.profile.name,
      members: session.state.workspaces
        .filter((open) => group.connections.includes(open.connection))
        .map((open) => ({
          id: open.connection,
          name: open.name,
          ...(readDisconnectCosts(open.connection) ?? {
            consoles: 0,
            unsaved: 0,
            running: 0,
          }),
          transaction: transactions[open.connection] ?? null,
        })),
    })
  }
  const confirm = async () => {
    if (!review || running.current) return
    const targets = review.members
    setReview(null)
    setResult(null)
    running.current = true
    setBusy(true)
    const closed: Array<string> = []
    try {
      for (const target of targets) {
        if (
          !session.state.workspaces.some(
            (open) => open.connection === target.id
          )
        )
          continue
        if (!(await reviewWorkspaceDisconnect(target.id))) break
        closed.push(target.name)
      }
    } catch (caught) {
      report(caught)
    } finally {
      setResult(
        `Disconnected: ${closed.join(", ") || "none"}. ${targets.length - closed.length} reviewed workspace(s) were not disconnected by this operation. Connections opened afterwards were untouched.`
      )
      running.current = false
      setBusy(false)
    }
  }
  return {
    request,
    busy,
    result,
    dismiss: () => setResult(null),
    element: review && (
      <GroupDisconnectDialog
        {...review}
        onCancel={() => setReview(null)}
        onReview={(id) => {
          setReview(null)
          showConnection(id)
        }}
        onConfirm={() => void confirm()}
      />
    ),
  }
}
