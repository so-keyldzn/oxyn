import * as React from "react"
import type { QueryClient } from "@tanstack/react-query"
import { useStore } from "@tanstack/react-store"

import { RESULT_PAGE_QUERY } from "@/components/oxyn/result-grid"
import { preferencesStore } from "@/features/settings/preferences"

/**
 * Reformats the result pages already held once a format change is saved.
 *
 * Pages are cached with no expiry: without this, a grid keeps the pages it
 * holds in the old format and fetches the next ones in the new, mixing both.
 * Invalidating refetches the visible pages from the retained buffer — no SQL
 * runs again (ADR-0012) — and the others when they are next shown.
 */
export function useResultFormatRefresh(queryClient: QueryClient) {
  const revision = useStore(preferencesStore, (state) => state.formatRevision)
  const seen = React.useRef(revision)
  React.useEffect(() => {
    if (seen.current === revision) return
    seen.current = revision
    void queryClient.invalidateQueries({ queryKey: [RESULT_PAGE_QUERY] })
  }, [queryClient, revision])
}
