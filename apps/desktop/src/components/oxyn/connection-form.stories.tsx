import type { Meta, StoryObj } from "@storybook/react-vite"
import { expect, fn, userEvent, waitFor } from "storybook/test"

import { ConnectionForm } from "./connection-form"
import { postgresDriver, savedConnections, sqliteDriver } from "./fixtures"
import type { ConnectionDetails } from "@/lib/ipc/settings"

const billing: ConnectionDetails = {
  ...savedConnections[0]!,
  values: {
    host: "db.internal",
    port: "5432",
    database: "billing",
    user: "app",
    sslmode: "require",
  },
  hasStoredSecrets: true,
}

const meta = {
  title: "Oxyn/ConnectionForm",
  component: ConnectionForm,
  decorators: [
    (Story) => (
      <div className="max-w-2xl p-6">
        <Story />
      </div>
    ),
  ],
  args: {
    driver: postgresDriver,
    onSubmit: fn(),
    onCancel: fn(),
    onAbort: fn(),
    onBrowse: fn(async () => "/Users/me/data/scratch.sqlite"),
  },
} satisfies Meta<typeof ConnectionForm>

export default meta
type Story = StoryObj<typeof meta>

async function fillPostgres(
  canvas: Parameters<NonNullable<Story["play"]>>[0]["canvas"]
) {
  await userEvent.type(canvas.getByLabelText(/^Name/), "billing")
  await userEvent.type(canvas.getByLabelText(/^Database/), "billing")
  await userEvent.type(canvas.getByLabelText(/^User/), "app")
  await userEvent.type(canvas.getByLabelText(/^Password/), "hunter2")
}

export const PostgreSQL: Story = {
  play: async ({ canvas, args }) => {
    // A new connection starts as production (I-02), Metadata (ADR-0006).
    await expect(
      canvas.getByRole("radio", { name: /PRODUCTION/ })
    ).toBeChecked()
    await expect(canvas.getByRole("radio", { name: /^Metadata/ })).toBeChecked()

    // Nothing is sent while a required field is empty, and the form says which.
    const connect = canvas.getByRole("button", { name: "Connect" })
    await expect(connect).toBeDisabled()
    await expect(
      canvas.getByText(/Required: Name, Database, User/)
    ).toBeVisible()
    await expect(canvas.getByLabelText(/^Database/)).toHaveAttribute(
      "aria-required",
      "true"
    )

    await fillPostgres(canvas)
    await userEvent.click(connect)
    await waitFor(() =>
      expect(args.onSubmit).toHaveBeenCalledWith(
        expect.objectContaining({
          environment: "production",
          privacyTier: "metadata",
          readOnly: false,
          secrets: { password: "hunter2" },
          values: expect.not.objectContaining({ password: expect.anything() }),
        })
      )
    )
  },
}

export const PortIsDigitsOnly: Story = {
  play: async ({ canvas }) => {
    await fillPostgres(canvas)
    const port = canvas.getByLabelText(/^Port/)
    await expect(port).toHaveAttribute("type", "text")
    await userEvent.clear(port)
    await userEvent.type(port, "54a2")
    await expect(canvas.getByText("Port is digits only.")).toBeVisible()
    await expect(canvas.getByRole("button", { name: "Connect" })).toBeDisabled()
  },
}

export const SQLite: Story = {
  args: { driver: sqliteDriver },
  play: async ({ canvas, args }) => {
    await userEvent.click(canvas.getByRole("button", { name: /Browse/ }))
    await waitFor(() =>
      expect(canvas.getByLabelText(/^Database file/)).toHaveValue(
        "/Users/me/data/scratch.sqlite"
      )
    )
    await expect(args.onBrowse).toHaveBeenCalled()
  },
}

export const OpeningCanBeCancelled: Story = {
  args: { submitting: true },
  play: async ({ canvas, args }) => {
    // In flight: the fields are inert, the only live action is Cancel.
    await expect(canvas.getByLabelText(/^Name/)).toBeDisabled()
    await expect(canvas.getByText("Opening the connection…")).toBeVisible()
    await userEvent.keyboard("{Escape}")
    await expect(args.onAbort).toHaveBeenCalledOnce()
    await userEvent.click(canvas.getByRole("button", { name: /Cancel/ }))
    await expect(args.onAbort).toHaveBeenCalledTimes(2)
    await expect(args.onSubmit).not.toHaveBeenCalled()
  },
}

export const Cancelling: Story = {
  args: { submitting: true, aborting: true },
}

export const ServerRefused: Story = {
  args: {
    error: {
      message:
        'opening a session on "billing": FATAL:  password authentication failed for user "app" (SQLSTATE 28P01)',
      retryable: false,
    },
  },
  play: async ({ canvas }) => {
    await expect(
      canvas.queryByRole("button", { name: "Connect again" })
    ).toBeNull()
  },
}

export const Retryable: Story = {
  args: {
    existing: billing,
    error: {
      message:
        'opening a session on "billing": connection to server at "db.internal", port 5432 failed: timeout expired',
      retryable: true,
    },
  },
  play: async ({ canvas, args }) => {
    await expect(
      canvas.getByText("This error is transient: trying again may succeed.")
    ).toBeVisible()
    await userEvent.click(canvas.getByRole("button", { name: "Save again" }))
    await waitFor(() => expect(args.onSubmit).toHaveBeenCalledOnce())
  },
}

export const ChangingThePrivacyTier: Story = {
  play: async ({ canvas }) => {
    // What leaves is shown before anything is saved.
    await expect(
      canvas.getByText("Row values, including server errors that quote one")
    ).toBeVisible()
    await userEvent.click(canvas.getByRole("radio", { name: /^Sampled/ }))
    await waitFor(() =>
      expect(
        canvas.getByText("Row samples you approve, column by column")
      ).toBeVisible()
    )
  },
}

export const EditingASavedConnection: Story = {
  args: { existing: billing },
  play: async ({ canvas, args }) => {
    await expect(canvas.getByLabelText(/^Name/)).toHaveValue("billing")
    await expect(canvas.getByLabelText(/^Host/)).toHaveValue("db.internal")
    // A stored secret is never pre-filled, nor required again (I-03).
    const password = canvas.getByLabelText(/^Password/)
    await expect(password).toHaveValue("")
    await expect(password).toHaveAttribute("type", "password")

    await userEvent.click(canvas.getByRole("switch", { name: "Read only" }))
    await userEvent.click(canvas.getByRole("button", { name: "Save" }))
    await waitFor(() =>
      expect(args.onSubmit).toHaveBeenCalledWith(
        expect.objectContaining({ readOnly: true, secrets: {} })
      )
    )
  },
}

export const HostileAndRightToLeftName: Story = {
  args: {
    existing: {
      ...billing,
      name: 'فواتير "users"; DROP TABLE audit; --',
    },
  },
}

export const Light: Story = { globals: { theme: "light" } }

export const Narrow: Story = {
  decorators: [
    (Story) => (
      <div className="w-[380px] p-3">
        <Story />
      </div>
    ),
  ],
}
