// Story and test data for the connection screens. Nothing here is shown in
// the application: the screens only draw what the backend returned.

import type { ConnectionSummary } from "@/lib/ipc/settings"

export const summaries: Array<ConnectionSummary> = [
  {
    id: "018f0000-0000-7000-8000-000000000001",
    name: "billing",
    driver: "postgres",
    driverName: "PostgreSQL",
    location: "db.internal:5432 / billing",
    environment: "production",
    readOnly: false,
    privacyTier: "metadata",
  },
  {
    id: "018f0000-0000-7000-8000-000000000002",
    name: "billing replica",
    driver: "postgres",
    driverName: "PostgreSQL",
    location: "replica.internal / billing",
    environment: "staging",
    readOnly: true,
    privacyTier: "local",
  },
  {
    id: "018f0000-0000-7000-8000-000000000003",
    name: "scratch",
    driver: "sqlite",
    driverName: "SQLite",
    location: "scratch.sqlite",
    environment: "local",
    readOnly: false,
    privacyTier: "sampled",
  },
]

/** Two connections with one name: only the location tells them apart. */
export const homonyms: Array<ConnectionSummary> = [
  { ...summaries[0]!, id: "018f0000-0000-7000-8000-00000000000a" },
  {
    ...summaries[0]!,
    id: "018f0000-0000-7000-8000-00000000000b",
    location: "db-eu.internal:5432 / billing",
    environment: "development",
  },
]

/** Names a user may legally give, and a server may legally hold. */
export const hostileNames: Array<ConnectionSummary> = [
  {
    ...summaries[0]!,
    id: "018f0000-0000-7000-8000-0000000000c1",
    name: `${"production-eu-west-analytics-warehouse-".repeat(4)}end`,
  },
  {
    ...summaries[1]!,
    id: "018f0000-0000-7000-8000-0000000000c2",
    name: '"users"; DROP TABLE audit; --<img src=x onerror=alert(1)>',
  },
  {
    ...summaries[2]!,
    id: "018f0000-0000-7000-8000-0000000000c3",
    name: "قاعدة الفواتير ‮txt.exe",
    location: null,
  },
]

/** A long-lived workspace: the list is filtered and folded past five. */
export const manyConnections: Array<ConnectionSummary> = Array.from(
  { length: 23 },
  (_, index): ConnectionSummary => {
    const base = summaries[index % summaries.length]!
    const suffix = String(index + 1).padStart(2, "0")
    return {
      ...base,
      id: `018f0000-0000-7000-8000-0000000001${suffix}`,
      name: `${base.name} ${suffix}`,
    }
  }
)
