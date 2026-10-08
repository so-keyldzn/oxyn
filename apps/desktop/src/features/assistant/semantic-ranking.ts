import { createStore } from "@tanstack/react-store"

import { BackendError } from "@/lib/ipc/client"
import { semantic } from "@/lib/ipc/semantic"
import type { SemanticSnapshot } from "@/lib/ipc/semantic"

// The webview's copy of semantic ranking (ADR-0056). Rust owns the state and
// pushes every change — download progress included — on
// `subscribe_semantic_ranking`; this store holds the last snapshot, and what
// the front alone knows: a request on its way, or one the backend refused.

export interface SemanticRankingView {
  /** `null` until the backend answered once. */
  snapshot: SemanticSnapshot | null
  pending: boolean
  refused: string | null
}

export const semanticRankingStore = createStore<SemanticRankingView>({
  snapshot: null,
  pending: false,
  refused: null,
})

function patch(next: Partial<SemanticRankingView>) {
  semanticRankingStore.setState((current) => ({ ...current, ...next }))
}

function message(error: unknown) {
  return error instanceof BackendError ? error.message : String(error)
}

let subscribed = false

/** Once per webview: the current state, then every change. */
export function followSemanticRanking() {
  if (subscribed) return
  subscribed = true
  // Subscribed first, and the read only seeds the store: its answer can land
  // after a newer progress message, and must not put an older state back.
  let heard = false
  void semantic
    .subscribe((snapshot) => {
      heard = true
      patch({ snapshot })
    })
    .catch((error: unknown) => {
      subscribed = false
      patch({ refused: message(error) })
    })
  void semantic
    .state()
    .then((snapshot) => {
      if (!heard) patch({ snapshot })
    })
    .catch((error: unknown) => {
      patch({ refused: message(error) })
    })
}

async function request(send: () => Promise<SemanticSnapshot>) {
  patch({ pending: true, refused: null })
  try {
    const snapshot = await send()
    patch({ snapshot, pending: false })
  } catch (error) {
    patch({ pending: false, refused: message(error) })
  }
}

export function enableSemanticRanking() {
  void request(semantic.enable)
}

export function disableSemanticRanking() {
  void request(semantic.disable)
}

/**
 * The assistant panel opened: the model loads now rather than inside the
 * first question's two seconds. A refusal costs that second, nothing else.
 */
export function preloadSemanticModel() {
  void semantic.preload().catch(() => undefined)
}
