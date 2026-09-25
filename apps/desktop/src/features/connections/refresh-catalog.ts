import { useQueryClient } from "@tanstack/react-query"

import { toast } from "@/components/ui/toast"
import { BackendError, newCommandId } from "@/lib/ipc/client"
import { metadata } from "@/lib/ipc/metadata"
import type { OpenConnection } from "@/lib/ipc/types"

/**
 * `Refresh catalog` of a saved connection's menu, wherever the list is: the
 * start screen or Settings. The whole catalog is read again through the bus,
 * and said in a toast — the workspace may be hidden, or behind the dialog.
 */
export function useRefreshCatalogOf() {
  const queryClient = useQueryClient()
  return async (open: OpenConnection) => {
    try {
      const outcome = await metadata.refreshCatalog(
        newCommandId(),
        open.connection,
        open.session,
        null
      )
      if (outcome.type === "denied")
        toast.add({
          title: "Catalog not refreshed",
          description: outcome.reason,
          type: "error",
        })
      else if (outcome.type === "catalogRefreshed")
        toast.add({
          title: `Catalog of ${open.name} refreshed`,
          type: "success",
        })
    } catch (error) {
      toast.add({
        title: "Catalog not refreshed",
        description:
          error instanceof BackendError || error instanceof Error
            ? error.message
            : String(error),
        type: "error",
      })
    } finally {
      // The workspace's tree reads it again, as after its own refresh.
      await queryClient.invalidateQueries({
        queryKey: ["catalog", open.connection],
      })
      await queryClient.invalidateQueries({
        queryKey: ["catalog-search", open.connection],
      })
    }
  }
}
