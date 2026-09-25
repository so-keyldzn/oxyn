import { useStore } from "@tanstack/react-store"

import { CloseWindowDialog } from "@/components/oxyn/close-window-dialog"
import {
  cancelWindowClose,
  closeWindow,
  windowClose,
} from "@/features/windows/window-close"

/**
 * The dialog of this window's close, when it is not the last and some of its
 * consoles would lose work (ADR-0043). Shown only while the close is decided:
 * a close that loses nothing goes without it.
 */
export function CloseWindowHost() {
  const close = useStore(windowClose)
  const asking = close !== null && close.rows.length > 0
  return (
    <CloseWindowDialog
      rows={asking ? close.rows : null}
      connections={close?.connections ?? []}
      busy={close?.busy ?? false}
      onCancel={cancelWindowClose}
      onClose={() => void closeWindow()}
    />
  )
}
