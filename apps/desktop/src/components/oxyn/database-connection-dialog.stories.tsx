import type { Meta, StoryObj } from "@storybook/react-vite"
import { expect, fn, waitFor } from "storybook/test"
import { DatabaseConnectionDialog } from "./database-connection-dialog"
import { postgresDriver, savedConnections } from "./fixtures"

const meta = {
  title: "Oxyn/DatabaseConnectionDialog",
  component: DatabaseConnectionDialog,
  args: {
    database: "audit",
    driver: postgresDriver,
    prefill: {
      name: "Audit",
      readOnly: false,
      values: {
        host: "localhost",
        port: "5432",
        database: "audit",
        user: "analyst",
        sslmode: "disable",
      },
    },
    candidates: [],
    submitting: false,
    error: null,
    saved: false,
    onSubmit: fn(),
    onChoose: fn(),
    onFresh: fn(),
    onClose: fn(),
    onCancel: fn(),
  },
} satisfies Meta<typeof DatabaseConnectionDialog>
export default meta
type Story = StoryObj<typeof meta>
export const NewAccount: Story = {
  play: async ({ canvasElement }) => {
    const { within } = await import("storybook/test")
    const canvas = within(canvasElement.ownerDocument.body)
    await expect(canvas.getByLabelText(/^Host/)).toBeDisabled()
    await expect(canvas.getByLabelText(/^Database/)).toBeDisabled()
    await expect(canvas.getByLabelText(/^User/)).toBeEnabled()
    await expect(canvas.getByLabelText(/^Password/)).toHaveValue("")
    await expect(
      canvas.getByRole("radio", { name: /PRODUCTION/ })
    ).toBeChecked()
  },
}
export const Opening: Story = { args: { submitting: true } }
export const SavedNotOpened: Story = {
  args: {
    saved: true,
    error: {
      message:
        "Example authentication refused. The saved identity remains available.",
      retryable: false,
    },
  },
}
export const Error: Story = {
  args: {
    error: {
      message: "Example seed changed. Reconnect the source first.",
      retryable: false,
    },
  },
}
export const Accounts: Story = {
  args: {
    candidates: [
      {
        ...savedConnections[0]!,
        id: "example-account-one",
        name: "Audit reader",
        driver: "postgres",
        values: { user: "reader" },
        hasStoredSecrets: true,
      },
      {
        ...savedConnections[0]!,
        id: "example-account-two",
        name: "Audit maintainer",
        driver: "postgres",
        values: { user: "maintainer" },
        hasStoredSecrets: true,
      },
    ],
  },
}

export const AccountFormScrollStaysAboveActions: Story = {
  play: async ({ canvasElement }) => {
    const { within } = await import("storybook/test")
    const dialog = within(canvasElement.ownerDocument.body).getByRole("dialog")
    const root = dialog.querySelector<HTMLElement>('[data-slot="scroll-area"]')
    const viewport = dialog.querySelector<HTMLElement>(
      '[data-slot="scroll-area-viewport"]'
    )
    const actions = dialog.querySelector<HTMLElement>(
      '[data-slot="connection-form-actions"]'
    )
    if (!root || !viewport || !actions)
      throw new globalThis.Error(
        "The account form needs bounded scrolling and persistent actions"
      )
    const footerTop = actions.getBoundingClientRect().top
    const footerOffset = actions.offsetTop
    expect(viewport.clientHeight).toBeGreaterThan(0)
    expect(viewport.getBoundingClientRect().height).toBeLessThanOrEqual(
      root.getBoundingClientRect().height + 1
    )
    expect(viewport.getBoundingClientRect().bottom).toBeLessThanOrEqual(
      footerTop + 1
    )
    viewport.scrollTop = viewport.scrollHeight
    const lastChoice = [
      ...viewport.querySelectorAll<HTMLElement>('[role="radio"]'),
    ].at(-1)
    if (!lastChoice)
      throw new globalThis.Error("Privacy choices must remain reachable")
    await waitFor(() => {
      const choice = lastChoice.getBoundingClientRect()
      const bounds = viewport.getBoundingClientRect()
      expect(choice.top).toBeGreaterThanOrEqual(bounds.top)
      expect(choice.bottom).toBeLessThanOrEqual(bounds.bottom)
      expect(actions.offsetTop).toBe(footerOffset)
    })
  },
}
