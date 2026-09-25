import { toast } from "@/components/ui/toast"
import { writeClipboard } from "@/lib/clipboard"

/**
 * Copies text the user asked for, and says whether it worked.
 *
 * Only what is on screen reaches this function — a name the backend quoted, a
 * definition, a value page. Never an identifier of a connection or a session:
 * the clipboard is one of the six channels a secret must not reach (I-03).
 *
 * Called synchronously from the click (`writeClipboard`). A failure — the
 * text that could not be read included — is a toast of the copy, never a
 * problem of the surface it was asked from.
 */
export async function copyToClipboard(
  text: string | Promise<string>,
  what: string
): Promise<boolean> {
  try {
    await writeClipboard(text)
    toast.add({ title: `${what} copied`, type: "success" })
    return true
  } catch (error) {
    toast.add({
      title: `${what} not copied`,
      description: error instanceof Error ? error.message : String(error),
      type: "error",
    })
    return false
  }
}
