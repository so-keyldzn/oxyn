import * as React from "react"

import {
  AlertDialog,
  AlertDialogCancel,
  AlertDialogContent,
  AlertDialogDescription,
  AlertDialogFooter,
  AlertDialogHeader,
  AlertDialogTitle,
} from "@/components/ui/alert-dialog"
import { Button } from "@/components/ui/button"
import { Spinner } from "@/components/ui/spinner"
import { EnvironmentBadge } from "@/components/oxyn/environment-badge"
import type { ApprovalPreview, Environment } from "@/lib/ipc/types"

export interface PendingApproval {
  command: string
  reason: string
  preview: ApprovalPreview | null
}

/** Who asked for the write: the person at the keyboard, or an agent. */
export type ApprovalActor = { kind: "human" } | { kind: "agent"; name: string }

// Lines one PageUp/PageDown moves: a screenful of the preview, minus one line
// of overlap so the reader keeps their place.
const PAGE_LINES = 12
const LINE_PX = 20

/**
 * The review of a write the policy held back.
 *
 * Built so it cannot be confirmed by reflex (docs/UX-SPEC.md, « Les opérations
 * destructrices ») : the action names the connection, `Cancel` takes the
 * initial focus and comes first at every width, Enter alone does not approve,
 * and Escape rejects.
 */
export function ApprovalDialog({
  approval,
  connectionName,
  environment,
  actor = { kind: "human" },
  deciding = false,
  onDecide,
}: {
  approval: PendingApproval | null
  connectionName: string
  environment: Environment
  actor?: ApprovalActor
  deciding?: boolean
  onDecide: (approved: boolean) => void
}) {
  const cancelRef = React.useRef<HTMLButtonElement>(null)
  const name = approval?.preview?.connection ?? connectionName
  // The weight of the action follows what is at stake: a production write is
  // destructive in look as in effect, a local one is not dressed as a threat.
  const destructive = environment === "production" || environment === "staging"

  return (
    <AlertDialog
      open={approval !== null}
      onOpenChange={(open) => {
        if (!open && !deciding) onDecide(false)
      }}
    >
      <AlertDialogContent
        // The base width is keyed on `data-size`, so a plain `max-w-xl` loses
        // to it; and a grid track sized by its content lets a long label widen
        // the footer past the frame. `grid-cols-1` bounds it.
        className="grid-cols-1 data-[size=default]:max-w-[calc(100%-2rem)] data-[size=default]:sm:max-w-xl"
        initialFocus={cancelRef}
        // Enter on the dialog body must never reach the destructive action.
        onKeyDown={(event) => {
          if (
            event.key === "Enter" &&
            !(event.target instanceof HTMLButtonElement)
          ) {
            event.preventDefault()
          }
        }}
      >
        <AlertDialogHeader>
          <div className="flex flex-wrap items-center gap-2">
            <AlertDialogTitle>Review before running</AlertDialogTitle>
            <EnvironmentBadge environment={environment} />
          </div>
          <AlertDialogDescription className="wrap-anywhere">
            {actor.kind === "agent" ? (
              <>
                <strong className="font-medium text-foreground">
                  An agent ({actor.name})
                </strong>{" "}
                is asking.{" "}
              </>
            ) : null}
            {approval?.reason} This statement will run on{" "}
            <strong className="font-medium text-foreground">
              <bdi>{name}</bdi>
            </strong>
            .
          </AlertDialogDescription>
        </AlertDialogHeader>

        {approval?.preview ? (
          // A native scroller: the keyboard scrolls it (arrows, Page, Home,
          // End) without extra code, which the custom scroll area did not.
          <pre
            data-selectable
            tabIndex={0}
            aria-label="Statement to approve"
            onKeyDown={(event) => {
              const target = event.currentTarget
              const page = PAGE_LINES * LINE_PX
              if (event.key === "PageDown") target.scrollTop += page
              else if (event.key === "PageUp") target.scrollTop -= page
              else return
              event.preventDefault()
            }}
            className="max-h-64 overflow-auto rounded-md border bg-card p-3 font-mono text-xs leading-5 break-words whitespace-pre-wrap outline-none focus-visible:ring-2 focus-visible:ring-ring"
          >
            {approval.preview.statement}
          </pre>
        ) : null}

        <p className="text-xs text-muted-foreground">
          {approval?.preview?.estimatedRows != null
            ? `Estimated rows affected: ${approval.preview.estimatedRows.toLocaleString("en-US")}`
            : "The number of affected rows is unknown."}
        </p>

        {/* `flex-col` rather than the footer's reversed stack: stacked on a
            narrow window, Cancel stays above the action. */}
        <AlertDialogFooter className="flex-col sm:flex-row">
          <AlertDialogCancel ref={cancelRef} disabled={deciding}>
            Cancel
          </AlertDialogCancel>
          <Button
            variant={destructive ? "destructive" : "default"}
            disabled={deciding}
            onClick={() => onDecide(true)}
            // A connection name is whatever the user typed: without a bound it
            // pushes the action past the dialog, and the label stops being
            // readable at all. The description above carries it in full.
            title={`Run on ${name}`}
            className="min-w-0 shrink"
          >
            {deciding ? <Spinner data-icon="inline-start" /> : null}
            <span className="min-w-0 truncate">
              Run on <bdi>{name}</bdi>
            </span>
          </Button>
        </AlertDialogFooter>
      </AlertDialogContent>
    </AlertDialog>
  )
}
