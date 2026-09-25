import { toast } from "@/components/ui/toast"

/**
 * A copy asked from one of the assistant's context menus.
 *
 * It goes through the view's `onCopy`, as its button's does (I-01), and says
 * when it worked: a button shows a tick in place, a menu closes and would
 * otherwise leave no trace. A failure is said by `onCopy` itself.
 */
export async function copyFromMenu(
  onCopy: (text: string) => Promise<boolean> | boolean,
  text: string,
  what: string
) {
  if (await onCopy(text))
    toast.add({ title: `${what} copied`, type: "success" })
}
