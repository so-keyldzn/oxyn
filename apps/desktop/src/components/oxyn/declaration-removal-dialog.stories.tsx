import type { Meta, StoryObj } from "@storybook/react-vite"
import { expect, fn, userEvent, waitFor, within } from "storybook/test"

import { DeclarationRemovalDialog } from "./declaration-removal-dialog"
import { externalAgent, remoteProvider } from "./assistant-fixtures"
import { expectContainedInFrame, openFrame } from "./frame-overflow"

const meta = {
  title: "Oxyn/Settings/DeclarationRemovalDialog",
  component: DeclarationRemovalDialog,
  args: {
    removal: { kind: "provider", provider: remoteProvider },
    onRemoveProvider: fn(),
    onRemoveAgent: fn(),
    onClose: fn(),
  },
} satisfies Meta<typeof DeclarationRemovalDialog>

export default meta
type Story = StoryObj<typeof meta>

export const Closed: Story = { args: { removal: null } }

/** A label with nothing to break on stays inside the frame. */
export const LongContentStaysInTheFrame: Story = {
  args: {
    removal: {
      kind: "agent",
      agent: {
        ...externalAgent,
        label: "codex_analytics_team_staging_workspace_agent_with_a_long_name",
      },
    },
  },
  play: async () => {
    await expectContainedInFrame(await openFrame("alert-dialog-content"))
  },
}

export const RemovingAProvider: Story = {
  play: async ({ args }) => {
    const body = within(document.body)
    const title = await body.findByText(
      "Remove the “Work account” declaration?"
    )
    await waitFor(() => expect(title).toBeVisible())
    await expect(body.getByText(/its stored key are removed/)).toBeVisible()
    await userEvent.click(body.getByRole("button", { name: "Remove" }))
    await expect(args.onRemoveProvider).toHaveBeenCalledWith(remoteProvider)
    await expect(args.onClose).toHaveBeenCalled()
  },
}

export const RemovingAnAgent: Story = {
  args: { removal: { kind: "agent", agent: externalAgent } },
  play: async ({ args }) => {
    const body = within(document.body)
    const title = await body.findByText(
      `Remove the “${externalAgent.label}” declaration?`
    )
    await waitFor(() => expect(title).toBeVisible())
    await userEvent.click(body.getByRole("button", { name: "Cancel" }))
    await expect(args.onRemoveAgent).not.toHaveBeenCalled()
    await expect(args.onClose).toHaveBeenCalled()
  },
}
