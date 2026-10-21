import * as React from "react"
import { useStore } from "@tanstack/react-store"

import { BackendErrorAlert } from "@/components/oxyn/backend-error-alert"
import { SemanticRankingSettings } from "@/components/oxyn/semantic-ranking-settings"
import { Spinner } from "@/components/ui/spinner"

import {
  disableSemanticRanking,
  enableSemanticRanking,
  followSemanticRanking,
  semanticRankingStore,
} from "./semantic-ranking"

/** Settings ▸ Semantic ranking, wired to the backend (ADR-0056). */
export function SemanticRankingSection() {
  const view = useStore(semanticRankingStore)

  React.useEffect(() => {
    followSemanticRanking()
  }, [])

  if (view.snapshot === null && view.refused !== null)
    return (
      <BackendErrorAlert
        title="The semantic ranking state could not be read"
        error={{ message: view.refused, retryable: false }}
        nextStep="Close and open the settings again; the option stays as it was saved."
      />
    )

  if (view.snapshot === null)
    return (
      <p className="flex items-center gap-2 text-sm text-muted-foreground">
        <Spinner aria-hidden /> Reading the semantic ranking state…
      </p>
    )

  return (
    <SemanticRankingSettings
      snapshot={view.snapshot}
      pending={view.pending}
      refused={view.refused}
      onEnable={enableSemanticRanking}
      onDisable={disableSemanticRanking}
    />
  )
}
