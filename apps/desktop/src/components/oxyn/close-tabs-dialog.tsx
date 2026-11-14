import { ScrollArea } from "@/components/ui/scroll-area"
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
import { Alert, AlertDescription } from "@/components/ui/alert"
import { Button } from "@/components/ui/button"
import { Spinner } from "@/components/ui/spinner"
import { closeMessage } from "@/features/consoles/console-model"
import type { TabCloseReview } from "@/features/workspace/use-close-tabs"

/** A single review names every console whose text or transaction is at risk. */
export function CloseTabsDialog({
  review,
  onCancel,
  onSave,
  onDiscard,
}: {
  review: TabCloseReview | null
  onCancel: () => void
  onSave: () => void
  onDiscard: () => void
}) {
  const cancel = React.useRef<HTMLButtonElement>(null)
  const blocked = review?.tabs.some((tab) => tab.blocked) ?? false
  return (
    <AlertDialog
      open={review !== null}
      onOpenChange={(open) => {
        if (!open) onCancel()
      }}
    >
      <AlertDialogContent
        initialFocus={cancel}
        className="grid-cols-1 data-[size=default]:sm:max-w-xl"
        onKeyDown={(event) => {
          if (
            event.key === "Enter" &&
            !(event.target instanceof HTMLButtonElement)
          )
            event.preventDefault()
        }}
      >
        <AlertDialogHeader>
          <AlertDialogTitle>
            Close {review?.tabs.length ?? 0} tabs?
          </AlertDialogTitle>
          <AlertDialogDescription>
            One choice applies to all listed tabs. Saving preserves query text;
            it does not commit transactions. Conflicted documents are saved as
            new copies.
          </AlertDialogDescription>
        </AlertDialogHeader>
        <ScrollArea className="max-h-64 min-h-0 rounded-md border [&>[data-slot=scroll-area-viewport]]:max-h-64">
          <ul
            tabIndex={0}
            aria-label="Tabs to close"
            className="flex flex-col gap-3 p-3 outline-none focus-visible:ring-2 focus-visible:ring-ring"
          >
            {review?.tabs.map((tab) => (
              <li
                key={tab.key}
                className="flex flex-col gap-1 text-sm wrap-anywhere"
              >
                <span className="font-medium">{tab.title}</span>
                <p className="text-xs text-muted-foreground">
                  {tab.blocked ??
                    (tab.reasons
                      ? closeMessage(tab.reasons)
                      : "Closing this tab does not change database data.")}
                </p>
              </li>
            ))}
          </ul>
        </ScrollArea>
        {review?.notice ? (
          <Alert variant="destructive">
            <AlertDescription className="whitespace-pre-wrap">
              {review.notice}
            </AlertDescription>
          </Alert>
        ) : null}
        {review?.busy ? (
          <p role="status" className="flex items-center gap-2 text-sm">
            <Spinner aria-hidden />
            Closing the selected tabs… Cancel stops pending work; completed
            closes cannot be undone here.
          </p>
        ) : null}
        <AlertDialogFooter className="flex-col sm:flex-row">
          <AlertDialogCancel ref={cancel}>Cancel</AlertDialogCancel>
          <Button
            variant="outline"
            disabled={review?.busy || blocked}
            onClick={onSave}
          >
            Save all and close
          </Button>
          <Button
            variant="destructive"
            disabled={review?.busy || blocked}
            onClick={onDiscard}
          >
            Discard all and close
          </Button>
        </AlertDialogFooter>
      </AlertDialogContent>
    </AlertDialog>
  )
}
