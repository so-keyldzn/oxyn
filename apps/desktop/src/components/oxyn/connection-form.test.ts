import { describe, expect, it } from "vitest"

import { draftFrom, missingFields, settingsChanged } from "./connection-form"
import type { DriverChoice } from "@/lib/ipc/types"

const postgres: DriverChoice = {
  id: "postgres",
  displayName: "PostgreSQL",
  family: "relational",
  defaultPort: 5432,
  fields: [
    {
      key: "host",
      label: "Host",
      kind: { type: "text" },
      required: true,
      secret: false,
      default: "localhost",
      help: null,
    },
    {
      key: "password",
      label: "Password",
      kind: { type: "password" },
      required: false,
      secret: true,
      default: null,
      help: null,
    },
    {
      key: "sslmode",
      label: "TLS",
      kind: { type: "choice", options: ["disable", "require"] },
      required: false,
      secret: false,
      default: null,
      help: null,
    },
  ],
}

describe("draftFrom", () => {
  it("sends secrets to the keyring side only", () => {
    const draft = draftFrom(postgres, {
      name: " prod ",
      environment: "production",
      privacyTier: "metadata",
      readOnly: false,
      values: { host: "db.internal", password: "hunter2", sslmode: "" },
    })
    expect(draft.values).toEqual({ host: "db.internal" })
    expect(draft.secrets).toEqual({ password: "hunter2" })
    expect(JSON.stringify(draft.values)).not.toContain("hunter2")
    expect(draft.name).toBe("prod")
  })

  it("drops an empty secret, so an edit keeps the stored one", () => {
    const draft = draftFrom(postgres, {
      name: "prod",
      environment: "production",
      privacyTier: "metadata",
      readOnly: false,
      values: { host: "db.internal", password: "" },
    })
    expect(draft.secrets).toEqual({})
  })

  it("keeps the marking the user chose, never a guess", () => {
    const draft = draftFrom(postgres, {
      name: "x",
      environment: "local",
      privacyTier: "local",
      readOnly: true,
      values: {},
    })
    expect(draft.environment).toBe("local")
    expect(draft.privacyTier).toBe("local")
    expect(draft.readOnly).toBe(true)
  })

  it("does not count a stored secret as missing", () => {
    const form = {
      name: "prod",
      environment: "production" as const,
      privacyTier: "metadata" as const,
      readOnly: false,
      values: { host: "", password: "" },
    }
    expect(missingFields(postgres, form)).toEqual(["Host"])
    expect(
      missingFields(postgres, form, {
        id: "x",
        name: "prod",
        driver: "postgres",
        environment: "production",
        readOnly: false,
        privacyTier: "metadata",
        values: {},
        hasStoredSecrets: true,
      })
    ).toEqual(["Host"])
  })
})

describe("settingsChanged", () => {
  const saved = {
    id: "x",
    name: "prod",
    driver: "postgres",
    environment: "production" as const,
    readOnly: false,
    privacyTier: "metadata" as const,
    values: { host: "db.internal", sslmode: "require" },
    hasStoredSecrets: true,
  }
  const form = (values: Record<string, string>) => ({
    name: "renamed",
    environment: "local" as const,
    privacyTier: "local" as const,
    readOnly: true,
    values,
  })

  it("ignores the name, the marking and a re-trimmed value", () => {
    expect(
      settingsChanged(
        postgres,
        form({ host: " db.internal ", sslmode: "require", password: "" }),
        saved
      )
    ).toBe(false)
  })

  it("sees a changed host or a weaker TLS mode", () => {
    expect(
      settingsChanged(
        postgres,
        form({ host: "db.other", sslmode: "require" }),
        saved
      )
    ).toBe(true)
    expect(
      settingsChanged(
        postgres,
        form({ host: "db.internal", sslmode: "disable" }),
        saved
      )
    ).toBe(true)
  })

  it("never counts a typed secret as a changed setting", () => {
    expect(
      settingsChanged(
        postgres,
        form({ host: "db.internal", sslmode: "require", password: "new" }),
        saved
      )
    ).toBe(false)
  })

  it("asks for a required stored secret again once a setting changed", () => {
    const required = {
      ...postgres,
      fields: postgres.fields.map((field) =>
        field.secret ? { ...field, required: true } : field
      ),
    }
    expect(
      missingFields(
        required,
        form({ host: "db.internal", sslmode: "require" }),
        saved
      )
    ).toEqual([])
    expect(
      missingFields(
        required,
        form({ host: "db.other", sslmode: "require" }),
        saved
      )
    ).toEqual(["Password"])
  })
})
