import { useQuery } from "@tanstack/react-query"

import { aiKeys } from "./queries"
import type { ProposalState } from "@/components/oxyn/assistant-proposal"
import { ai } from "@/lib/ipc/ai"
import type { ProposalTarget } from "@/lib/ipc/ai"
import type { CatalogAddress } from "@/lib/ipc/types"

/**
 * The schema-change template for a column or a named constraint (ADR-0025).
 *
 * Composed by the backend from the local catalog, identifiers quoted for the
 * dialect. `unavailable` when the dialect can change nothing there: the caller
 * then shows no `Propose change…` button at all.
 */
export function useSchemaProposal(
  connection: string,
  address: CatalogAddress,
  target: ProposalTarget | null
): ProposalState {
  const query = useQuery({
    queryKey: [...aiKeys.all, "proposal", connection, address, target],
    queryFn: () =>
      target === null
        ? Promise.resolve(null)
        : ai.proposeSchemaChange(connection, address, target),
    enabled: target !== null,
    // The catalog may change under it; a template is cheap to compose again.
    staleTime: 0,
  })
  if (query.isError) return { status: "error", message: query.error.message }
  if (query.isPending) return { status: "loading" }
  return query.data
    ? { status: "ready", ...query.data }
    : { status: "unavailable" }
}
