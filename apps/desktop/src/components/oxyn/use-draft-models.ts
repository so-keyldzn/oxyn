import * as React from "react"

import { probeOf } from "@/components/oxyn/model-picker-model"
import type {
  DraftModels,
  ProbeInputs,
} from "@/components/oxyn/model-picker-model"
import type { ModelListing, ModelProbe } from "@/lib/ipc/ai"

/** The backend call, passed down by the feature; stories pass a mock. */
export type ListDraftModels = (
  probe: ModelProbe,
  refresh: boolean
) => Promise<ModelListing>

export type ConnectionTest =
  | { status: "idle" }
  | { status: "testing" }
  | { status: "ok"; count: number }
  | { status: "failed"; message: string }

/** Long enough not to ask the provider once per keystroke of a URL. */
export const LIST_DEBOUNCE_MS = 400

function messageOf(error: unknown) {
  return error instanceof Error ? error.message : String(error)
}

/**
 * The model list of the provider being typed.
 *
 * Lists by itself once the form knows enough (`probeOf`, strict), after a
 * pause in typing. A Tauri call cannot be aborted: each request takes a number,
 * and only the latest one may write — an answer for the endpoint typed before
 * is dropped, never shown for the one typed after. Changing the kind, the
 * endpoint or the key drops the list shown.
 *
 * The key only travels inside the probe: it is never logged nor rendered, and
 * no cache of the screen is keyed on it (I-03). The last probe listed is kept,
 * by reference, only to tell a new key from the same one.
 */
export function useDraftModels(
  inputs: ProbeInputs | null,
  list: ListDraftModels | undefined
) {
  const [models, setModels] = React.useState<DraftModels>({ status: "idle" })
  const [test, setTest] = React.useState<ConnectionTest>({ status: "idle" })
  const latest = React.useRef(0)
  const testTurn = React.useRef(0)

  const kind = inputs?.kind
  const endpoint = inputs?.endpoint
  const key = inputs?.key
  const editing = inputs?.editing ?? null
  const auto = React.useMemo(
    () =>
      kind === undefined || endpoint === undefined || key === undefined
        ? null
        : probeOf({ kind, endpoint, key, editing }, true),
    [kind, endpoint, key, editing]
  )
  const manual = React.useMemo(
    () =>
      kind === undefined || endpoint === undefined || key === undefined
        ? null
        : probeOf({ kind, endpoint, key, editing }, false),
    [kind, endpoint, key, editing]
  )

  const request = React.useEffectEvent(
    async (probe: ModelProbe, refresh: boolean): Promise<DraftModels> => {
      const turn = ++latest.current
      setModels({ status: "loading" })
      let answer: DraftModels
      try {
        answer = list
          ? await list(probe, refresh)
          : { status: "error", message: "Listing models is not available." }
      } catch (error) {
        answer = { status: "error", message: messageOf(error) }
      }
      if (turn !== latest.current) return { status: "loading" }
      setModels(answer)
      return answer
    }
  )

  // Keyed on the fields, not on `auto`: a remote endpoint without a key keeps
  // `auto` at `null` while a click lists it anyway, and that answer must not
  // survive the next endpoint.
  React.useEffect(() => {
    // Whatever was in flight answers a question nobody asks any more.
    latest.current += 1
    setModels({ status: "idle" })
    setTest({ status: "idle" })
  }, [kind, endpoint, key])

  // The backend caches a list by kind and endpoint, never by key: after a new
  // key, the cached list is the one the previous key was allowed to see.
  const listedWith = React.useRef<ModelProbe | null>(null)

  React.useEffect(() => {
    const turn = ++latest.current
    if (auto === null || list === undefined) {
      setModels({ status: "idle" })
      return
    }
    setModels({ status: "loading" })
    const timer = window.setTimeout(() => {
      if (turn !== latest.current) return
      const newKey =
        listedWith.current !== null && listedWith.current.key !== auto.key
      listedWith.current = auto
      void request(auto, newKey)
    }, LIST_DEBOUNCE_MS)
    return () => window.clearTimeout(timer)
  }, [auto, list])

  // Leaving the form drops the answers still on their way.
  React.useEffect(
    () => () => {
      latest.current += 1
    },
    []
  )

  const refresh = () => {
    if (manual !== null) void request(manual, true)
  }

  const testConnection = () => {
    if (manual === null) return
    setTest({ status: "testing" })
    const pending = request(manual, true)
    // `request` numbers itself before its first wait: this is its turn.
    const turn = latest.current
    testTurn.current = turn
    void pending.then((answer) => {
      // A later test speaks for itself; a later listing only ends this one.
      if (testTurn.current !== turn) return
      setTest(
        answer.status === "ok"
          ? { status: "ok", count: answer.models.length }
          : answer.status === "loading" || answer.status === "idle"
            ? { status: "idle" }
            : { status: "failed", message: answer.message }
      )
    })
  }

  return {
    models,
    test,
    /** Whether a click can ask: an endpoint is known. */
    canAsk: manual !== null && list !== undefined,
    refresh,
    testConnection,
  }
}
