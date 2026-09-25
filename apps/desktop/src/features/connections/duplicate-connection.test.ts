import { describe, expect, it } from "vitest"

import { postgresDriver } from "@/components/oxyn/fixtures"
import { duplicatePrefill } from "@/features/connections/duplicate-connection"
import type { ConnectionDetails } from "@/lib/ipc/settings"

const PASSWORD = "hunter2-do-not-leak"

/** A source whose values carry what they never should: a secret, a stray key. */
const source = {
  id: "018f0000-0000-7000-8000-000000000001",
  name: "billing",
  driver: "postgres",
  environment: "staging",
  readOnly: true,
  privacyTier: "sampled",
  values: {
    host: "db.internal",
    port: "5432",
    database: "billing",
    user: "app",
    password: PASSWORD,
    secretRef: "oxyn/keyring-entry",
  },
  hasStoredSecrets: true,
} as ConnectionDetails

describe("duplicatePrefill", () => {
  it("carries the name, suffixed, and the declared non-secret parameters", () => {
    expect(duplicatePrefill(postgresDriver, source)).toEqual({
      name: "billing copy",
      readOnly: true,
      values: {
        host: "db.internal",
        port: "5432",
        database: "billing",
        user: "app",
      },
    })
  })

  it("never carries a secret, a secret's reference or the marking", () => {
    const prefill = duplicatePrefill(postgresDriver, source)
    const text = JSON.stringify(prefill)
    expect(text).not.toContain(PASSWORD)
    expect(text).not.toContain("keyring")
    expect(text).not.toContain(source.id)
    // A copy starts as a new connection: production, default tier (I-02).
    expect(prefill).not.toHaveProperty("environment")
    expect(prefill).not.toHaveProperty("privacyTier")
  })
})
