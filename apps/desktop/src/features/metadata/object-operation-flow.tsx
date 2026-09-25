import * as React from "react"

import { ApprovalDialog } from "@/components/oxyn/approval-dialog"
import type { PendingApproval } from "@/components/oxyn/approval-dialog"
import { ObjectOperationReviewDialog } from "@/components/oxyn/object-operation-review"
import type {
  DependentsRead,
  ReviewPhase,
} from "@/components/oxyn/object-operation-review"
import { openInConsole } from "@/features/consoles/open-in-console"
import { backend, newCommandId } from "@/lib/ipc/client"
import { metadata } from "@/lib/ipc/metadata"
import { objectOperations } from "@/lib/ipc/object-operations"
import type {
  ObjectOperation,
  ObjectOperationReview,
  OperationKind,
} from "@/lib/ipc/object-operations"
import type { CatalogNode, OpenConnection } from "@/lib/ipc/types"

interface Target {
  node: CatalogNode
  operation: OperationKind
}

function messageOf(caught: unknown) {
  return caught instanceof Error ? caught.message : String(caught)
}

function operationOf(
  kind: OperationKind,
  cascade: boolean,
  newName: string
): ObjectOperation {
  switch (kind) {
    case "drop":
      return { kind, cascade }
    case "truncate":
      return { kind, cascade }
    case "rename":
      return { kind, column: null, newName }
  }
}

/**
 * `Drop…`, `Truncate…` and `Rename…` from the catalog: the review, then — when
 * the gate holds the statement — `ApprovalDialog` in the same place, never a
 * second modal on top (ADR-0042).
 *
 * The statement is composed by the backend at every change of the box; the
 * catalog is invalidated by the executor after a DDL, and read again by the
 * sidebar's refresh signal. Nothing here is optimistic: the tree changes once
 * the server has answered.
 */
export function useObjectOperation(
  open: OpenConnection,
  onApplied: (node: CatalogNode, operation: OperationKind) => void
) {
  const [target, setTarget] = React.useState<Target | null>(null)
  const [cascade, setCascade] = React.useState(false)
  const [newName, setNewName] = React.useState("")
  const [review, setReview] = React.useState<ObjectOperationReview | null>(null)
  const [problem, setProblem] = React.useState<string | null>(null)
  const [dependents, setDependents] = React.useState<DependentsRead>({
    kind: "read",
  })
  const [phase, setPhase] = React.useState<ReviewPhase>({ kind: "idle" })
  const [approval, setApproval] = React.useState<PendingApproval | null>(null)
  const [deciding, setDeciding] = React.useState(false)
  const running = React.useRef<string | null>(null)
  // The latest composition wins: a keystroke answered late must not show a
  // statement for a name no longer in the field.
  const asked = React.useRef(0)

  const compose = React.useCallback(
    async (current: Target, operation: ObjectOperation, readKeys: boolean) => {
      asked.current += 1
      const ticket = asked.current
      try {
        const composed = await objectOperations.review(
          open.connection,
          open.session,
          current.node.address,
          operation
        )
        if (ticket !== asked.current) return
        setReview(composed)
        setProblem(null)
        const facet = composed.dependents
        if (
          readKeys &&
          facet.type === "incomingKeys" &&
          facet.freshness.state !== "fetched"
        ) {
          setDependents({ kind: "reading" })
          try {
            const outcome = await metadata.refreshRelationFacet(
              newCommandId(),
              open.connection,
              open.session,
              current.node.address,
              "incomingKeys"
            )
            if (outcome.type !== "catalogRefreshed")
              throw new Error("The foreign keys were not read.")
            const reread = await objectOperations.review(
              open.connection,
              open.session,
              current.node.address,
              operation
            )
            if (ticket === asked.current) setReview(reread)
            setDependents({ kind: "read" })
          } catch (caught) {
            setDependents({ kind: "failed", message: messageOf(caught) })
          }
        }
      } catch (caught) {
        if (ticket !== asked.current) return
        setProblem(messageOf(caught))
      }
    },
    [open.connection, open.session]
  )

  const start = React.useCallback(
    (node: CatalogNode, operation: OperationKind) => {
      const current = { node, operation }
      setTarget(current)
      // Unticked at every opening, and never remembered (ADR-0042).
      setCascade(false)
      setNewName("")
      setReview(null)
      setProblem(null)
      setDependents({ kind: "read" })
      setPhase({ kind: "idle" })
      void compose(current, operationOf(operation, false, ""), true)
    },
    [compose]
  )

  const close = () => {
    setTarget(null)
    setApproval(null)
    setReview(null)
  }

  const finish = (current: Target) => {
    close()
    onApplied(current.node, current.operation)
  }

  const run = async () => {
    if (!target || !review) return
    const id = newCommandId()
    running.current = id
    setPhase({ kind: "running" })
    try {
      const outcome = await objectOperations.run(
        id,
        open.connection,
        target.operation,
        review.sql
      )
      switch (outcome.type) {
        case "applied":
          finish(target)
          return
        case "needsApproval":
          setPhase({ kind: "idle" })
          setApproval({
            command: outcome.command,
            reason: outcome.reason,
            preview: outcome.preview,
          })
          return
        case "denied":
          setPhase({ kind: "denied", reason: outcome.reason })
          return
        case "failed":
          setPhase({ kind: "failed", message: outcome.message })
          return
        case "ambiguous":
          setPhase({ kind: "ambiguous", message: outcome.message })
          return
        case "notSent":
          setPhase({ kind: "notSent", message: outcome.message })
          return
      }
    } catch (caught) {
      // The backend answers every refusal before sending as `notSent`: an
      // error here is a lost or unreadable answer, possibly after the
      // statement ran. Never offered again (I-13).
      setPhase({ kind: "ambiguous", message: messageOf(caught) })
    } finally {
      running.current = null
    }
  }

  const decide = async (approved: boolean) => {
    if (!target || !approval) return
    if (!approved) {
      setDeciding(true)
      await backend.decide(approval.command, false).catch(() => undefined)
      setDeciding(false)
      close()
      return
    }
    setDeciding(true)
    try {
      const outcome = await backend.decide(approval.command, true)
      setApproval(null)
      if (outcome.type === "executed") finish(target)
      else if (outcome.type === "denied")
        setPhase({ kind: "denied", reason: outcome.reason })
      else if (outcome.type === "cancelled")
        setPhase({ kind: "ambiguous", message: "Stopped after it was sent." })
      else close()
    } catch (caught) {
      // `decide` does not say whether the error is ambiguous: the box never
      // offers to run again, and points to the catalog (I-13).
      setApproval(null)
      setPhase({ kind: "failed", message: messageOf(caught) })
    } finally {
      setDeciding(false)
    }
  }

  const parent = target ? { ...target.node.address, relation: null } : null

  const element = target ? (
    <>
      <ObjectOperationReviewDialog
        open={approval === null}
        operation={target.operation}
        review={review}
        problem={problem}
        newName={newName}
        onNewNameChange={(name) => {
          setNewName(name)
          void compose(
            target,
            operationOf(target.operation, cascade, name),
            false
          )
        }}
        cascade={cascade}
        onCascadeChange={(checked) => {
          setCascade(checked)
          void compose(
            target,
            operationOf(target.operation, checked, newName),
            false
          )
        }}
        dependents={dependents}
        phase={phase}
        onRun={() => void run()}
        onStop={() => {
          if (running.current)
            void backend.cancel(running.current).catch(() => undefined)
        }}
        onCancel={close}
        onRefreshCatalog={() => {
          if (parent)
            void metadata
              .refreshCatalog(
                newCommandId(),
                open.connection,
                open.session,
                parent
              )
              .catch(() => undefined)
          close()
        }}
        onOpenInConsole={() => {
          if (review)
            openInConsole(review.sql, {
              newConsole: true,
              notice:
                "Refused by the server from the catalog review · nothing has run from this console",
            })
          close()
        }}
      />
      <ApprovalDialog
        approval={approval}
        connectionName={open.name}
        environment={open.environment}
        deciding={deciding}
        onDecide={(approved) => void decide(approved)}
      />
    </>
  ) : null

  return { start, element }
}
