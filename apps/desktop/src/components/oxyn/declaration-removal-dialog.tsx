import * as React from "react"

import { removalQuestion } from "@/components/oxyn/provider-settings-model"
import {
  AlertDialog,
  AlertDialogAction,
  AlertDialogCancel,
  AlertDialogContent,
  AlertDialogDescription,
  AlertDialogFooter,
  AlertDialogHeader,
  AlertDialogTitle,
} from "@/components/ui/alert-dialog"
import type { DeclaredProvider, ExternalAgent } from "@/lib/ipc/ai"

export type Removal =
  | { kind: "provider"; provider: DeclaredProvider }
  | { kind: "agent"; agent: ExternalAgent }

/** Asks before a declaration is removed, naming it. */
export function DeclarationRemovalDialog({
  removal,
  onRemoveProvider,
  onRemoveAgent,
  onClose,
}: {
  /** What to remove; `null` closes the dialog. */
  removal: Removal | null
  onRemoveProvider: (provider: DeclaredProvider) => void
  onRemoveAgent: (agent: ExternalAgent) => void
  onClose: () => void
}) {
  // Kept while the dialog closes, so its title never empties mid-animation.
  const [shown, setShown] = React.useState<Removal | null>(removal)
  if (removal !== null && removal !== shown) setShown(removal)

  return (
    <AlertDialog
      open={removal !== null}
      onOpenChange={(open) => {
        if (!open) onClose()
      }}
    >
      <AlertDialogContent>
        <AlertDialogHeader>
          <AlertDialogTitle>
            {shown
              ? removalQuestion(
                  shown.kind === "provider"
                    ? shown.provider.label
                    : shown.agent.label
                )
              : ""}
          </AlertDialogTitle>
          <AlertDialogDescription>
            {shown?.kind === "provider"
              ? "The declaration and its stored key are removed. Documents keep the provenance they were written with."
              : "The declaration is removed. The program itself is left untouched."}
          </AlertDialogDescription>
        </AlertDialogHeader>
        <AlertDialogFooter>
          <AlertDialogCancel>Cancel</AlertDialogCancel>
          <AlertDialogAction
            variant="destructive"
            onClick={() => {
              if (removal?.kind === "provider")
                onRemoveProvider(removal.provider)
              if (removal?.kind === "agent") onRemoveAgent(removal.agent)
              onClose()
            }}
          >
            Remove
          </AlertDialogAction>
        </AlertDialogFooter>
      </AlertDialogContent>
    </AlertDialog>
  )
}
