import type * as React from "react"
import type { Meta, StoryObj } from "@storybook/react-vite"
import { expect, fn, userEvent, waitFor, within } from "storybook/test"

import { ConnectionScreenView } from "./connection-screen-view"
import { homonyms, summaries } from "@/components/oxyn/connection-fixtures"
import { postgresDriver, sqliteDriver } from "@/components/oxyn/fixtures"
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
    await expect(canvas.getByText("No saved connection")).toBeVisible()
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
    await userEvent.keyboard("{Meta>}[[{/Meta}")
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

    // The Back button and ⌘[ ask the same question.
    await userEvent.click(canvas.getByRole("button", { name: /^Back/ }))
    await userEvent.click(
      await body.findByRole("button", { name: "Keep editing" })
    )
    await waitFor(() => expect(body.queryByRole("alertdialog")).toBeNull())
    await userEvent.keyboard("{Meta>}[[{/Meta}")
    await userEvent.click(await body.findByRole("button", { name: "Discard" }))
    await expect(args.onLeaveDriver).toHaveBeenCalledOnce()
  },
}

export const ShortWindow: Story = {
  decorators: [sized("h-[600px] w-[1280px]")],
  args: { driver: sqliteDriver, onReturnToWorkspace: fn() },
  play: async ({ canvasElement }) => {
    await reachConnect(canvasElement, sqliteDriver)
    // The footnote is still there, below both columns.
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
    // Each column scrolls on its own.
    const scrollers = Array.from(
      canvasElement.querySelectorAll<HTMLElement>("[role=region] *")
    ).filter(
      (element) =>
        element.scrollHeight > element.clientHeight + 1 &&
        getComputedStyle(element).overflowY === "auto"
    )
    await expect(scrollers.length).toBeGreaterThanOrEqual(1)
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
