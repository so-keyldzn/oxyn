import * as React from "react"

/**
 * A draft with its `secrets` field removed from the type, not just the value.
 *
 * A plain `Omit<T, "secrets">` still lets a caller pass a value that carries
 * `secrets` — TypeScript allows excess properties on a variable of a wider
 * type. `secrets?: never` forbids the key outright, so `mutate(draft)` with a
 * full draft fails to typecheck: the type system is the guard against a
 * secret slipping back into a mutation's variables.
 */
export type WithoutSecrets<T extends { secrets: Record<string, string> }> =
  Omit<T, "secrets"> & { secrets?: never }

/**
 * Keeps a draft's secrets out of TanStack Query's `MutationCache`.
 *
 * A `Mutation` stays in the cache — variables included — after it settles,
 * for as long as its `useMutation` observer is attached (`gcTime` only
 * shortens the window that follows unmount). Both connection screens stay
 * mounted on the case that matters most: a failed attempt, shown from
 * `mutation.error`, where `reset()` cannot run without erasing that error.
 *
 * `hold` sets a draft's secrets aside and returns the rest; `take`, given that
 * same object back from inside `mutationFn`, reads them once and forgets
 * them. The secrets never become part of a mutation's variables, so there is
 * nothing for the cache or the React Query Devtools to retain (I-03).
 *
 * The secrets are keyed by the returned object, not held in a single slot:
 * a double submit calls `hold` twice before the first `mutationFn` runs
 * (`isPending` is only observed a tick later), and a single slot would send
 * the second attempt with `secrets: {}`. A `WeakMap` is also invisible to
 * serialization, so the devtools cannot show it either.
 */
export function useTypedSecrets() {
  const held = React.useRef(new WeakMap<object, Record<string, string>>())

  const hold = React.useCallback(
    <T extends { secrets: Record<string, string> }>(
      draft: T
    ): WithoutSecrets<T> => {
      const { secrets, ...rest } = draft
      held.current.set(rest, secrets)
      return rest
    },
    []
  )

  const take = React.useCallback((withoutSecrets: object) => {
    const secrets = held.current.get(withoutSecrets) ?? {}
    held.current.delete(withoutSecrets)
    return secrets
  }, [])

  return { hold, take }
}
