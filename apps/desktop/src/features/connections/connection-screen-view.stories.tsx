import type * as React from "react"
import type { Meta, StoryObj } from "@storybook/react-vite"
import { expect, fn, userEvent, waitFor, within } from "storybook/test"

import { ConnectionScreenView } from "./connection-screen-view"
import {
  homonyms,
  manyConnections,
  summaries,
} from "@/components/oxyn/connection-fixtures"
import {
  catalogueDrivers,
  postgresDriver,
  sqliteDriver,
} from "@/components/oxyn/fixtures"
import { modKey } from "@/lib/actions/platform"
import type { DriverChoice, FormField } from "@/lib/ipc/types"

const optional = (
  field: Omit<FormField, "required" | "secret" | "default" | "help">
): FormField => ({
  required: false,
  secret: false,
  default: null,
  help: null,
  ...field,
})

/** A driver that declares many fields: the form must scroll, never clip. */
const postgresWithSsl: DriverChoice = {
  ...postgresDriver,
  fields: [
    // `postgresDriver` already declares `sslmode`: a driver never declares a
    // key twice, and a fixture that did rendered two fields under one key.
    ...postgresDriver.fields,
    optional({
      key: "sslrootcert",
      label: "Root certificate",
      kind: { type: "path" },
    }),
    optional({
      key: "sslcert",
      label: "Client certificate",
      kind: { type: "path" },
    }),
    optional({ key: "sslkey", label: "Client key", kind: { type: "path" } }),
    optional({
      key: "application_name",
      label: "Application name",
      kind: { type: "text" },
    }),
    optional({
      key: "connect_timeout",
      label: "Connect timeout (s)",
      kind: { type: "number" },
    }),
    optional({
      key: "search_path",
      label: "Search path",
      kind: { type: "text" },
    }),
  ],
}

function sized(className: string) {
  return (Story: () => React.ReactElement) => (
    <div className={className}>
      <Story />
    </div>
  )
}

/**
 * Fills what the driver requires, then Tabs to Connect: it must be reached by
 * the keyboard and shown inside the screen, however short the window.
 */
async function reachConnect(canvasElement: HTMLElement, driver: DriverChoice) {
  const canvas = within(canvasElement)
  await userEvent.type(canvas.getByLabelText(/^Name/), "billing")
  for (const field of driver.fields) {
    if (!field.required || field.default) continue
    await userEvent.type(
      canvasElement.querySelector<HTMLInputElement>(`#field-${field.key}`)!,
      field.kind.type === "path" ? "/tmp/billing.sqlite" : "billing"
    )
  }
  const connect = canvas.getByRole("button", { name: "Connect" })
  await expect(connect).toBeEnabled()
  canvas.getByLabelText(/^Name/).focus()
  for (let step = 0; step < 60 && document.activeElement !== connect; step++) {
    await userEvent.tab()
  }
  await expect(connect).toHaveFocus()
  const screen = canvasElement.querySelector("main")!.getBoundingClientRect()
  await waitFor(() => {
    const rect = connect.getBoundingClientRect()
    expect(rect.top).toBeGreaterThanOrEqual(screen.top)
    expect(rect.bottom).toBeLessThanOrEqual(screen.bottom + 1)
  })
}

const meta = {
  title: "Screens/ConnectionScreen",
  component: ConnectionScreenView,
  decorators: [
    (Story) => (
      <div className="h-[820px]">
        <Story />
      </div>
    ),
  ],
  args: {
    connections: summaries,
    drivers: [postgresDriver, sqliteDriver],
    driver: null,
    onChooseDriver: fn(),
    onLeaveDriver: fn(),
    opening: null,
    onOpen: fn(),
    onRetryOpen: fn(),
    submitting: false,
    onSubmit: fn(),
    testing: false,
    testResult: null,
    onTest: fn(),
    onDraftChange: fn(),
    cancelling: false,
    onCancelOpening: fn(),
    approval: null,
    deciding: false,
    onDecide: fn(),
    onOpenLocalWork: fn(),
    onRetryConnections: fn(),
    onRetryDrivers: fn(),
  },
} satisfies Meta<typeof ConnectionScreenView>

export default meta
type Story = StoryObj<typeof meta>

export const FirstLaunch: Story = {
  args: { connections: [] },
  play: async ({ canvas, canvasElement }) => {
    // No workspace stayed open: the return action is absent, not disabled.
    await expect(
      canvas.queryByRole("button", { name: /Return to workspace/ })
    ).toBeNull()
    // Nothing to reopen: no empty list, the database types are the screen.
    await expect(
      canvas.getByRole("heading", { name: "Connect your first database" })
    ).toBeVisible()
    await expect(
      canvas.queryByRole("region", { name: "Saved connections" })
    ).toBeNull()
    await expect(
      canvas.getByRole("button", { name: /PostgreSQL/ })
    ).toBeEnabled()
    // One mark only, in the title bar.
    await expect(canvasElement.querySelectorAll("img")).toHaveLength(1)
  },
}

export const WorkspaceLeftOpen: Story = {
  args: { onReturnToWorkspace: fn() },
  play: async ({ canvas, args }) => {
    await expect(
      canvas.getByRole("button", { name: /Return to workspace/ })
    ).toBeVisible()
    await userEvent.keyboard("{Escape}")
    await expect(args.onReturnToWorkspace).toHaveBeenCalledOnce()
    await userEvent.keyboard(`{${modKey}>}[[{/${modKey}}`)
    await expect(args.onReturnToWorkspace).toHaveBeenCalledTimes(2)
  },
}

export const Loading: Story = {
  args: { connections: undefined, drivers: undefined },
}

export const OpeningASavedConnection: Story = {
  args: { opening: summaries[0]!.id, onReturnToWorkspace: fn() },
  play: async ({ canvas, args }) => {
    // Esc cancels the opening; it does not leave for the workspace.
    await userEvent.keyboard("{Escape}")
    await expect(args.onCancelOpening).toHaveBeenCalledOnce()
    await expect(args.onReturnToWorkspace).not.toHaveBeenCalled()
    await expect(
      canvas.getByRole("button", { name: /Return to workspace/ })
    ).toBeDisabled()
  },
}

export const OpenFailedRetryable: Story = {
  args: {
    openError: {
      message:
        'opening a session on "billing": connection to server at "db.internal", port 5432 failed: timeout expired',
      retryable: true,
    },
  },
  play: async ({ canvas, args }) => {
    await userEvent.click(canvas.getByRole("button", { name: "Try again" }))
    await expect(args.onRetryOpen).toHaveBeenCalledOnce()
  },
}

export const OpenFailedForGood: Story = {
  args: {
    openError: {
      message:
        'opening a session on "billing": FATAL:  database "billing" does not exist (SQLSTATE 3D000)',
      retryable: false,
    },
  },
}

export const NewConnection: Story = {
  args: { driver: postgresDriver },
  play: async ({ canvas, args }) => {
    // Nothing typed: every way back leaves at once, without a question.
    const back = canvas.getByRole("button", { name: /^Back/ })
    back.focus()
    await expect(back).toHaveFocus()
    await userEvent.keyboard("{Escape}")
    await expect(args.onLeaveDriver).toHaveBeenCalledOnce()
    await userEvent.click(canvas.getByRole("button", { name: "Connections" }))
    await expect(args.onLeaveDriver).toHaveBeenCalledTimes(2)
    await expect(
      within(canvas.getByRole("navigation")).getByText(
        "New PostgreSQL connection"
      )
    ).toBeVisible()
  },
}

export const LeavingTypedValuesAsksFirst: Story = {
  args: { driver: sqliteDriver },
  play: async ({ canvas, args }) => {
    const body = within(document.body)
    const name = canvas.getByLabelText(/^Name/)
    await userEvent.type(name, "billing")

    // Esc from the field asks; « Keep editing » keeps what was typed.
    await userEvent.keyboard("{Escape}")
    await userEvent.click(
      await body.findByRole("button", { name: "Keep editing" })
    )
    await expect(args.onLeaveDriver).not.toHaveBeenCalled()
    await expect(name).toHaveValue("billing")

    // The Back button and Mod+[ ask the same question.
    await userEvent.click(canvas.getByRole("button", { name: /^Back/ }))
    await userEvent.click(
      await body.findByRole("button", { name: "Keep editing" })
    )
    await waitFor(() => expect(body.queryByRole("alertdialog")).toBeNull())
    await userEvent.keyboard(`{${modKey}>}[[{/${modKey}}`)
    await userEvent.click(await body.findByRole("button", { name: "Discard" }))
    await expect(args.onLeaveDriver).toHaveBeenCalledOnce()
  },
}

export const ShortWindow: Story = {
  decorators: [sized("h-[600px] w-[1280px]")],
  args: { driver: sqliteDriver, onReturnToWorkspace: fn() },
  play: async ({ canvasElement }) => {
    await reachConnect(canvasElement, sqliteDriver)
    // The footnote is still there, below the form.
    await expect(
      within(canvasElement).getByText(
        /Secrets are stored in the system keyring/
      )
    ).toBeVisible()
  },
}

export const ManyFieldsShortWindow: Story = {
  decorators: [sized("h-[600px] w-[1280px]")],
  args: { driver: postgresWithSsl, connections: [...summaries, ...homonyms] },
  play: async ({ canvasElement }) => {
    await reachConnect(canvasElement, postgresWithSsl)
    // The page scrolls as a whole; nothing inside it clips the form.
    const main = canvasElement.querySelector("main")!
    await expect(main.scrollHeight).toBeGreaterThan(main.clientHeight)
  },
}

/** The status bar never claims more than it knows (UX-SPEC). */
async function expectNeverConnected(canvasElement: HTMLElement) {
  await expect(canvasElement.textContent).not.toMatch(/\bConnected\b/)
}

export const NotTested: Story = {
  args: { driver: sqliteDriver },
  play: async ({ canvas, canvasElement, args }) => {
    const status = canvas.getByRole("status", { name: "Connection status" })
    await expect(status).toHaveTextContent("Unnamed connection")
    await expect(status).toHaveTextContent("Not tested")
    const testAction = canvas.getByRole("button", { name: "Test" })
    await expect(testAction).toBeDisabled()

    // The bar names the connection being prepared, as it is typed.
    await userEvent.type(canvas.getByLabelText(/^Name/), "billing")
    await expect(status).toHaveTextContent("billing")
    await userEvent.type(
      canvasElement.querySelector<HTMLInputElement>("#field-path")!,
      "/tmp/billing.sqlite"
    )
    await expect(args.onDraftChange).toHaveBeenCalled()

    // Testing sends the draft; it neither saves nor opens it.
    await userEvent.click(testAction)
    await expect(args.onTest).toHaveBeenCalledWith(
      expect.objectContaining({ name: "billing", driver: sqliteDriver.id })
    )
    await expect(args.onSubmit).not.toHaveBeenCalled()
    await expect(status).toHaveTextContent("Not tested")
    await expectNeverConnected(canvasElement)
  },
}

export const Testing: Story = {
  args: { driver: sqliteDriver, testing: true },
  play: async ({ canvas, canvasElement, args }) => {
    await expect(
      canvas.getByRole("status", { name: "Connection status" })
    ).toHaveTextContent("Testing…")
    // A test is cancelled like an opening: Esc reaches the backend.
    await userEvent.keyboard("{Escape}")
    await expect(args.onCancelOpening).toHaveBeenCalledOnce()
    await expect(args.onLeaveDriver).not.toHaveBeenCalled()
    await expect(canvas.getByRole("button", { name: "Connect" })).toBeDisabled()
    await expectNeverConnected(canvasElement)
  },
}

export const TestPassed: Story = {
  args: {
    driver: sqliteDriver,
    testResult: { type: "succeeded", elapsedMs: 42 },
  },
  play: async ({ canvas, canvasElement }) => {
    const status = canvas.getByRole("status", { name: "Connection status" })
    await expect(status).toHaveTextContent("Test passed in 42 ms")
    await expect(status).not.toHaveTextContent("Not tested")
    // A passed test is not a session: Connect is still what saves and opens.
    await expect(canvas.getByRole("button", { name: "Connect" })).toBeVisible()
    await expectNeverConnected(canvasElement)
  },
}

export const TestFailed: Story = {
  args: {
    driver: postgresDriver,
    testResult: {
      type: "failed",
      message:
        'driver `postgres` (permanent error): FATAL:  password authentication failed for user "billing" (SQLSTATE 28P01)',
      class: "permanent",
      retryable: false,
    },
  },
  play: async ({ canvas, canvasElement }) => {
    const status = canvas.getByRole("status", { name: "Connection status" })
    await expect(status).toHaveTextContent("Test failed")
    // The server's words, code included: the audience reads them.
    await expect(status).toHaveTextContent(/SQLSTATE 28P01/)
    await expectNeverConnected(canvasElement)
  },
}

export const CreatingAndCancelling: Story = {
  args: { driver: postgresDriver, submitting: true },
  play: async ({ canvas, args }) => {
    await userEvent.click(canvas.getByRole("button", { name: /^Cancel/ }))
    await expect(args.onCancelOpening).toHaveBeenCalled()
  },
}

export const ProductionApproval: Story = {
  args: {
    driver: postgresDriver,
    submitting: true,
    approval: {
      command: "018f0000-0000-7000-8000-00000000c0de",
      reason: 'DDL operation on "billing", marked production',
      preview: {
        statement: "CreateConnection",
        connection: "billing",
        estimatedRows: null,
      },
      name: "billing",
      environment: "production",
    },
  },
  play: async () => {
    const dialog = within(document.body)
    const cancel = await dialog.findByRole("button", { name: "Cancel" })
    await waitFor(() => expect(cancel).toHaveFocus())
  },
}

export const Homonyms: Story = { args: { connections: homonyms } }

export const ListsFailed: Story = {
  args: {
    connections: undefined,
    drivers: undefined,
    connectionsError: {
      message: "listing the saved connections: database is locked",
      retryable: true,
    },
    driversError: {
      message:
        "The Oxyn backend is not available here (list_drivers): open the desktop application.",
      retryable: false,
    },
  },
}

export const Light: Story = {
  args: { onReturnToWorkspace: fn() },
  globals: { theme: "light" },
}

export const NarrowWindow: Story = {
  decorators: [sized("h-[700px] w-[420px]")],
  args: { onReturnToWorkspace: fn(), driver: postgresWithSsl },
  play: async ({ canvasElement }) => {
    await reachConnect(canvasElement, postgresWithSsl)
  },
}

/** A long-lived workspace on a build with the whole catalogue. */
export const BusyWorkspace: Story = {
  args: { connections: manyConnections, drivers: catalogueDrivers },
}

/** The whole catalogue on a first launch: the search is the screen. */
export const FirstLaunchWholeCatalogue: Story = {
  args: { connections: [], drivers: catalogueDrivers },
}
