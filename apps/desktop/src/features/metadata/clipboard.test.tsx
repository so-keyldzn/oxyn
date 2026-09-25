import { QueryClient, QueryClientProvider } from "@tanstack/react-query"
import { cleanup, fireEvent, render, screen } from "@testing-library/react"
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest"

import { ObjectInspectorPanel } from "@/features/metadata/inspectors"
import { inspectObject } from "@/features/metadata/inspection"
import type { RelationFacets } from "@/lib/ipc/metadata"
import type { CatalogAddress, OpenConnection } from "@/lib/ipc/types"

// I-03 sentinel sweep, channel 5 (clipboard). The other four channels are
// covered elsewhere: crates/oxyn-store/src/sentinel_tests.rs (workspace file)
// for the first, and crates/oxyn-desktop/src/sentinel_tests.rs for the other
// three — no_sentinel_reaches_an_error_shown_to_the_front (rendered IPC
// error), no_sentinel_reaches_the_journal (log) and
// no_sentinel_reaches_the_prompt_sent_to_a_provider (assembled AI prompt).
//
// No "connection copy" exists in this codebase: connection-screen-view.tsx,
// connection-form.tsx, saved-connections.tsx and connection-manager.tsx call
// neither `copyToClipboard` nor `navigator.clipboard.writeText`, and every
// secret field is a never-prefilled `input type="password"`
// (connection-form.tsx:494). So there is nothing to copy *from* a connection
// draft or a saved connection today.
//
// What an open connection does let a user copy is metadata the backend
// already quoted — the object inspector's "Copy qualified name" button
// (this test), and, through the same `copyToClipboard` helper
// (src/features/metadata/clipboard.ts), a relation's qualified name from the
// catalog sidebar (catalog-sidebar.tsx:189, same field, same guard — not
// duplicated here), a constraint definition, a DDL statement and a cell's
// full value (object-view.tsx, value-inspection.tsx). None of those carry a
// connection or session identifier, by the helper's own contract ("never an
// identifier of a connection or a session", clipboard.ts:7) — this test
// exercises the identifier-shaped one, the qualified name, because that is
// the field most likely to be widened into leaking one by accident.
//
// Three other call sites write through `writeClipboard` (src/lib/clipboard.ts)
// without `copyToClipboard`'s toast: assistant-panel.tsx copies an assistant
// message, result-grid.tsx the selected cell values, sql-editor.tsx the
// selected text. They copy what the user wrote or the query returned, never
// a connection or session identifier — out of scope for this file.

// A password chosen to be unmistakable if it ever leaks into JSON.
const SENTINEL = "s3cr3t-sentinel"

// The one field `ObjectInspector`'s copy button sends to the clipboard: quoted
// by the backend's dialect and copied as is (inspectors.tsx:36, object-inspector.tsx).
// Deliberately not SENTINEL: it is the legitimate payload this test proves
// reaches the clipboard, while every other text field on the fixture is the
// sentinel and must not.
const QUALIFIED_NAME = '"public"."customers"'

const relationFacets = vi.hoisted(() => vi.fn())
const toastAdd = vi.hoisted(() => vi.fn())

vi.mock("@/lib/ipc/metadata", () => ({
  metadata: { relationFacets },
}))

vi.mock("@/components/ui/toast", () => ({
  toast: { add: toastAdd },
}))

const facets: RelationFacets = {
  address: { catalog: SENTINEL, namespace: SENTINEL, relation: SENTINEL },
  kind: SENTINEL,
  holdsRecords: true,
  qualifiedName: QUALIFIED_NAME,
  detail: { freshness: { state: "never" }, value: null },
  indexes: null,
  foreignKeys: null,
  constraints: { freshness: { state: "never" }, value: null },
  incomingKeys: { freshness: { state: "never" }, value: null },
  definition: { freshness: { state: "never" }, value: null },
  uniqueKey: null,
}

const open: OpenConnection = {
  connection: SENTINEL,
  session: SENTINEL,
  name: SENTINEL,
  driver: "postgresql",
  environment: "development",
  readOnly: false,
  privacyTier: "metadata",
  capabilities: [],
  console: {
    session: SENTINEL,
    capabilities: [],
    readOnly: false,
    transactionState: "unknown",
  },
}

const address: CatalogAddress = {
  catalog: null,
  namespace: "public",
  relation: "customers",
}

let written: Array<string>

// jsdom 30.0.1 does not implement the Clipboard API: `navigator.clipboard` is
// `undefined` unless stubbed here, in this test only.
beforeEach(() => {
  written = []
  relationFacets.mockReset().mockResolvedValue(facets)
  toastAdd.mockReset()
  Object.defineProperty(navigator, "clipboard", {
    value: {
      writeText: vi.fn(async (text: string) => {
        written.push(text)
      }),
    },
    configurable: true,
  })
})

afterEach(() => {
  cleanup()
  inspectObject(null)
  Reflect.deleteProperty(navigator, "clipboard")
})

describe("copying from an open connection's object inspector", () => {
  it("copies the qualified name only, never the connection or session that opened it", async () => {
    inspectObject({ connection: open.connection, address })

    const client = new QueryClient({
      defaultOptions: { queries: { retry: false } },
    })
    render(
      <QueryClientProvider client={client}>
        <ObjectInspectorPanel open={open} />
      </QueryClientProvider>
    )

    const copyButton = await screen.findByRole("button", {
      name: "Copy qualified name",
    })
    fireEvent.click(copyButton)

    await vi.waitFor(() => expect(written).toHaveLength(1))

    expect(written).toEqual([QUALIFIED_NAME])
    expect(JSON.stringify(written)).not.toContain(SENTINEL)

    await vi.waitFor(() => expect(toastAdd).toHaveBeenCalled())
    expect(JSON.stringify(toastAdd.mock.calls)).not.toContain(SENTINEL)
  })
})
