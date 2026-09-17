import { toast } from "@/components/ui/toast"

/**
 * Copies text the user asked for, and says whether it worked.
 *
 * Only what is on screen reaches this function — a name the backend quoted, a
 * definition, a value page. Never an identifier of a connection or a session:
 * the clipboard is one of the six channels a secret must not reach (I-03).
 */
export async function copyToClipboard(text: string, what: string) {
  try {
    await navigator.clipboard.writeText(text)
    toast.add({ title: `${what} copied`, type: "success" })
  } catch (error) {
    toast.add({
      title: `${what} not copied`,
      description: error instanceof Error ? error.message : String(error),
      type: "error",
    })
  }
}
