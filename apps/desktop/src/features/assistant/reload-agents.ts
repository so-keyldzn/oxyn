import { useMutation, useQueryClient } from "@tanstack/react-query"

import { aiKeys } from "./queries"
import { ai } from "@/lib/ipc/ai"

/**
 * Reads the user's agent files again for the picker of `connection`.
 *
 * The answer replaces this connection's list at once; the lists of the other
 * connections are only marked stale, since the catalog they come from is the
 * same and was just rebuilt. A question already running keeps its agent;
 * the next one reads the new files (ADR-0049 § 6).
 */
export function useReloadAgents(connection: string) {
  const queryClient = useQueryClient()
  const reload = useMutation({
    mutationFn: () => ai.reloadAgents(connection),
    onSuccess: async (agents) => {
      queryClient.setQueryData(aiKeys.roles(connection), agents)
      await queryClient.invalidateQueries({
        queryKey: ["ai", "roles"],
        predicate: (query) => query.queryKey[2] !== connection,
      })
    },
  })
  return {
    reload: () => reload.mutate(),
    reloading: reload.isPending,
    error: reload.error
      ? reload.error instanceof Error
        ? reload.error.message
        : String(reload.error)
      : null,
  }
}
