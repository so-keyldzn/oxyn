import { useRouter } from "@tanstack/react-router"
import type { ErrorComponentProps } from "@tanstack/react-router"

import { RouteFailure } from "@/components/oxyn/route-failure"
import { flushAllDrafts } from "@/features/consoles/draft-registry"

function messageOf(error: unknown) {
  return error instanceof Error ? error.message : String(error)
}

/** A route failed to render: said, with a reload and a way back. */
export function RouteError({ error, reset }: ErrorComponentProps) {
  const router = useRouter()
  return (
    <RouteFailure
      kind="error"
      message={messageOf(error)}
      onReload={() => {
        // Drafts typed in the last idle delay are written before the page goes.
        void flushAllDrafts().finally(() => window.location.reload())
      }}
      onBack={() => {
        void router.navigate({ to: "/" }).then(reset)
      }}
    />
  )
}

export function RouteNotFound() {
  const router = useRouter()
  return (
    <RouteFailure
      kind="notFound"
      onBack={() => void router.navigate({ to: "/" })}
    />
  )
}
