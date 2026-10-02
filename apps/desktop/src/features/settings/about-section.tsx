import { useQuery } from "@tanstack/react-query"

import { AboutSettings } from "@/components/oxyn/about-settings"
import type { ThirdPartyLicensesState } from "@/components/oxyn/about-settings"
import { aboutBackend } from "@/lib/ipc/about"
import { loadThirdPartyLicenses } from "@/lib/third-party-licenses"

/**
 * The About section, with the identity and the notices of this build. The
 * notices are part of the
 * bundle and never change while the app runs: read once, never refetched,
 * never retried.
 */
export function AboutSection() {
  const query = useQuery({
    queryKey: ["third-party-licenses"],
    queryFn: loadThirdPartyLicenses,
    staleTime: Infinity,
    retry: false,
  })

  const licenses: ThirdPartyLicensesState = query.isPending
    ? { status: "loading" }
    : query.isError
      ? { status: "error", message: query.error.message }
      : query.data === null
        ? { status: "missing" }
        : { status: "ready", notices: query.data }

  // Fixed at compile time: read once, like the notices. A failure leaves the
  // lines « Unknown » rather than hiding them.
  const identity = useQuery({
    queryKey: ["build-identity"],
    queryFn: aboutBackend.buildIdentity,
    staleTime: Infinity,
    retry: false,
  })

  return <AboutSettings identity={identity.data ?? null} licenses={licenses} />
}
