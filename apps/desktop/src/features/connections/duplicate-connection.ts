import { NOT_COPIED } from "@/components/oxyn/connection-form"
import type { ConnectionPrefill } from "@/components/oxyn/connection-form"
import type { ConnectionDetails } from "@/lib/ipc/settings"
import type { DriverChoice } from "@/lib/ipc/types"

/**
 * The new connection form's starting values for a copy of `source`.
 *
 * Only what the driver declares as a non-secret field is carried: the
 * backend already leaves secrets out of `values`, and this does not rely on
 * it (I-03). The environment, the privacy tier and the sqlite-vec switch are
 * not carried — a copy starts in production, with the default tier and
 * sqlite-vec off, like any new connection (ADR-0054).
 */
export function duplicatePrefill(
  driver: DriverChoice,
  source: ConnectionDetails
): ConnectionPrefill {
  const values: Record<string, string> = {}
  for (const field of driver.fields) {
    if (field.secret || NOT_COPIED.has(field.key)) continue
    const value = source.values[field.key]
    if (value !== undefined) values[field.key] = value
  }
  return { name: `${source.name} copy`, readOnly: source.readOnly, values }
}
