import type {
  NativeJob,
  NativeTool,
  NativeReview,
} from "@/lib/ipc/native-transfers"
import type { NativeTransferViewState } from "./native-transfer-model"

export const nativeToolFixture: NativeTool = {
  grant: "00000000-0000-4000-8000-000000000001",
  identity: {
    engine: "postgres",
    role: "export",
    name: "pg_dump",
    version: "17.12",
  },
}
export const nativeJobFixture = (
  overrides: Partial<NativeJob> = {}
): NativeJob => ({
  version: 1,
  id: "00000000-0000-4000-8000-000000000002",
  connection: "example-source",
  session: "example-owner",
  owner: "example-window",
  command: null,
  connectionName: "Example development",
  database: "example",
  engine: "postgres",
  operation: "export",
  format: "customArchive",
  noOwner: overrides.operation === "restore",
  noPrivileges: overrides.operation === "restore",
  tool:
    overrides.operation === "restore"
      ? { ...nativeToolFixture.identity, role: "restore", name: "pg_restore" }
      : nativeToolFixture.identity,
  fileName: "example.dump",
  stage: "finished",
  outcome: "succeeded",
  failure: null,
  bytes: 8192,
  mayHaveChanged: false,
  reconciled: false,
  createdAt: "2026-11-15T00:00:00Z",
  finishedAt: "2026-11-15T00:00:01Z",
  ...overrides,
})
export const nativeReviewFixture: NativeReview = {
  job: nativeJobFixture({
    stage: "ready",
    outcome: null,
    bytes: 0,
    finishedAt: null,
  }),
  destination: "database example on 127.0.0.1:5432 as example_user",
}
export const nativeViewFixture = (
  overrides: Partial<NativeTransferViewState> = {}
): NativeTransferViewState => ({
  support: {
    engine: "postgres",
    export: true,
    restore: true,
    reason: null,
    exportFormats: ["customArchive", "sql"],
    restoreFormats: ["customArchive"],
  },
  operation: "export",
  format: "customArchive",
  tool: null,
  file: null,
  review: null,
  jobs: [],
  historyKnown: true,
  reading: false,
  busy: null,
  cancelling: false,
  noOwner: true,
  noPrivileges: true,
  trustedArchive: false,
  error: null,
  ...overrides,
})
