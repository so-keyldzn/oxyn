import { useQuery } from "@tanstack/react-query"

import type { DestinationOption } from "./availability"
import { modelsQuery } from "./queries"

/**
 * The models the selected provider lists, asked for as soon as the panel
 * shows that provider — not when its model menu opens: the reasoning-effort
 * selector depends on this list, and a control that only appears after
 * opening another menu is one nobody finds.
 *
 * Once per provider: the query never goes stale, so reopening the panel does
 * not ask again, and it is not retried (the client's `retry: false`). An
 * unusable provider — refused by the tier — is not asked at all. The panel
 * turns the query's state into the header's « Listing models… » and its
 * failure, in the backend's words (`ModelListState`).
 */
export function useProviderModels(selected: DestinationOption | null) {
  const provider =
    selected?.kind === "provider" && selected.usable ? selected.id : null
  return useQuery({
    ...modelsQuery(provider ?? ""),
    enabled: provider !== null,
  })
}
