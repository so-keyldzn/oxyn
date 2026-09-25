import { describe, expect, it } from "vitest"

import { connectionText } from "@/features/connections/copy-connection"
import type { ConnectionSummary } from "@/lib/ipc/settings"

const ID = "018f0000-0000-7000-8000-000000000001"
const PASSWORD = "hunter2-do-not-leak"
const SECRET_REF = "oxyn/018f0000-keyring-entry"

/**
 * A summary carrying what a future backend might add to it — a password, a
 * secret's reference, raw parameters. None of it may reach the clipboard.
 */
const withSecrets = {
  id: ID,
  name: "billing",
  driver: "postgres",
  driverName: "PostgreSQL",
  location: "db.internal:5432 / billing",
  environment: "production",
  readOnly: true,
  privacyTier: "metadata",
  password: PASSWORD,
  secretRef: SECRET_REF,
  values: { password: PASSWORD, user: "app" },
  secrets: { password: PASSWORD },
} as ConnectionSummary

describe("connectionText", () => {
  it("carries what the list shows", () => {
    expect(connectionText(withSecrets)).toBe(
      [
        "Name: billing",
        "Driver: PostgreSQL",
        "Environment: production",
        "Location: db.internal:5432 / billing",
        "Read only: yes",
      ].join("\n")
    )
  })

  it("never carries a secret, its reference or the connection's id", () => {
    const text = connectionText(withSecrets)
    expect(text).not.toContain(PASSWORD)
    expect(text).not.toContain(SECRET_REF)
    expect(text).not.toContain(ID)
    expect(text).not.toContain("018f0000")
  })

  it("leaves out a location the summary does not have", () => {
    const text = connectionText({
      ...withSecrets,
      location: null,
      readOnly: false,
    })
    expect(text).toBe(
      "Name: billing\nDriver: PostgreSQL\nEnvironment: production"
    )
  })
})
