import { QueryClient, QueryClientProvider } from "@tanstack/react-query"
import {
  cleanup,
  fireEvent,
  render,
  screen,
  waitFor,
} from "@testing-library/react"
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest"

import { postgresDriver } from "@/components/oxyn/fixtures"
import type { ConnectionDetails, ConnectionEdit } from "@/lib/ipc/settings"
import type { ConnectionDraft, DriverChoice } from "@/lib/ipc/types"
import { ConnectionScreen } from "./connection-screen"

// A password chosen to be unmistakable if it is ever sent again.
const SENTINEL = "s3cr3t-sentinel"

// Required, so that « typed again » is something the form can refuse.
const driver: DriverChoice = {
  ...postgresDriver,
  fields: postgresDriver.fields.map((field) =>
    field.secret ? { ...field, required: true } : field
  ),
}

const saved = {
  id: "018f0000-0000-7000-8000-0000000b1111",
  name: "billing",
  driver: driver.id,
  environment: "production" as const,
  readOnly: false,
  privacyTier: "metadata" as const,
}

const ipc = vi.hoisted(() => ({
  connect: vi.fn(),
  reconnect: vi.fn(),
  connectionDetails: vi.fn(),
  updateConnection: vi.fn(),
}))

vi.mock("@/lib/ipc/client", () => ({
  BackendError: class BackendError extends Error {
    retryable = false
  },
  backend: {
    listDrivers: () => Promise.resolve([driver]),
    connect: ipc.connect,
    reconnect: ipc.reconnect,
    cancel: () => Promise.resolve(),
    disconnect: () => Promise.resolve(),
  },
  newCommandId: () => crypto.randomUUID(),
}))
vi.mock("@/lib/ipc/settings", () => ({
  settingsBackend: {
    listConnections: () => Promise.resolve([]),
    connectionDetails: ipc.connectionDetails,
    updateConnection: ipc.updateConnection,
  },
}))
vi.mock("@tanstack/react-router", () => ({
  useNavigate: () => () => Promise.resolve(),
}))
vi.mock("@tauri-apps/plugin-dialog", () => ({ open: vi.fn() }))

/** What the backend stored from the first attempt: its parameters, a secret. */
function storedFrom(draft: ConnectionDraft): ConnectionDetails {
  return { ...saved, values: draft.values, hasStoredSecrets: true }
}

beforeEach(() => {
  // jsdom lays nothing out: the form brings its error into view regardless.
  Element.prototype.scrollIntoView = vi.fn()
  ipc.connect.mockImplementation((_command: string, draft: ConnectionDraft) => {
    ipc.connectionDetails.mockResolvedValue(storedFrom(draft))
    return Promise.resolve({
      type: "saved",
      connection: saved,
      message: 'opening a session on "billing": connection refused',
      retryable: false,
    })
  })
  ipc.updateConnection.mockResolvedValue({
    type: "saved",
    connection: saved,
    secretsError: null,
  })
  ipc.reconnect.mockRejectedValue(new Error("connection refused"))
})

afterEach(() => {
  cleanup()
  vi.clearAllMocks()
})

async function failFirstOpening() {
  render(
    <QueryClientProvider
      client={
        new QueryClient({ defaultOptions: { queries: { retry: false } } })
      }
    >
      <ConnectionScreen />
    </QueryClientProvider>
  )
  fireEvent.click(await screen.findByRole("button", { name: /PostgreSQL/ }))
  type(screen.getByLabelText(/^Name/), "billing")
  type(screen.getByLabelText(/^Database/), "billing")
  type(screen.getByLabelText(/^User/), "app")
  type(screen.getByLabelText(/^Password/), SENTINEL)
  fireEvent.click(screen.getByRole("button", { name: "Connect" }))
  await screen.findByText("Saved, but not connected")
  expect(ipc.connect).toHaveBeenCalledTimes(1)
  expect(ipc.connect.mock.calls[0]?.[1]?.secrets).toEqual({
    password: SENTINEL,
  })
}

function valueOf(field: HTMLElement) {
  return (field as HTMLInputElement).value
}

function type(field: HTMLElement, value: string) {
  fireEvent.change(field, { target: { value } })
}

function lastEdit(): ConnectionEdit {
  const call = ipc.updateConnection.mock.calls.at(-1)
  return call?.[2] as ConnectionEdit
}

describe("ConnectionScreen after a saved connection failed to open", () => {
  it("does not send the first attempt's secret to a changed destination", async () => {
    await failFirstOpening()
    const password = screen.getByLabelText(/^Password/)
    await waitFor(() => expect(valueOf(password)).toBe(""))

    type(screen.getByLabelText(/^Host/), "elsewhere.example")

    // Asked again: the stored secret was typed for the former host.
    await screen.findByText(
      /Connection settings changed — enter the password again/
    )
    const connect = screen.getByRole<HTMLButtonElement>("button", {
      name: "Connect",
    })
    expect(connect.disabled).toBe(true)
    fireEvent.click(connect)
    expect(ipc.updateConnection).not.toHaveBeenCalled()

    type(password, "typed-for-elsewhere")
    await waitFor(() => expect(connect.disabled).toBe(false))
    fireEvent.click(connect)
    await waitFor(() => expect(ipc.updateConnection).toHaveBeenCalledTimes(1))
    expect(lastEdit().values.host).toBe("elsewhere.example")
    expect(lastEdit().secrets).toEqual({ password: "typed-for-elsewhere" })
    expect(JSON.stringify(ipc.updateConnection.mock.calls)).not.toContain(
      SENTINEL
    )
    expect(ipc.connect).toHaveBeenCalledTimes(1)
  })

  it("retries the same details on the saved connection, keeping its secret", async () => {
    await failFirstOpening()
    const first = ipc.connect.mock.calls[0]?.[1] as ConnectionDraft
    await waitFor(() =>
      expect(valueOf(screen.getByLabelText(/^Password/))).toBe("")
    )

    const connect = screen.getByRole<HTMLButtonElement>("button", {
      name: "Connect",
    })
    await waitFor(() => expect(connect.disabled).toBe(false))
    fireEvent.click(connect)

    await waitFor(() => expect(ipc.reconnect).toHaveBeenCalledTimes(1))
    // One profile: the saved one is edited, never a second one created.
    expect(ipc.connect).toHaveBeenCalledTimes(1)
    expect(ipc.updateConnection).toHaveBeenCalledTimes(1)
    expect(ipc.updateConnection.mock.calls[0]?.[1]).toBe(saved.id)
    expect(ipc.reconnect.mock.calls[0]?.[1]).toBe(saved.id)
    // Nothing else differs, and the stored secret is kept, not sent again.
    expect(lastEdit()).toEqual({
      name: first.name,
      environment: first.environment,
      privacyTier: first.privacyTier,
      readOnly: first.readOnly,
      values: first.values,
      secrets: {},
    })
  })
})
