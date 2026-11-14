import type { Meta, StoryObj } from "@storybook/react-vite"
import { expect, fn, screen, userEvent, waitFor, within } from "storybook/test"
import { SchemaSamplingDialog } from "./schema-sampling-dialog"

const meta = {
  title: "Oxyn/SchemaSamplingDialog",
  component: SchemaSamplingDialog,
  args: {
    connection: "Example development",
    object: "orders",
    limits: {
      maxDocuments: 1000,
      maxRetainedBytes: 8 * 1024 * 1024,
      maxDurationMs: 10000,
    },
    onSample: fn(),
    onCancel: fn(),
  },
} satisfies Meta<typeof SchemaSamplingDialog>
export default meta
type Story = StoryObj<typeof meta>
export const CancelIsDefault: Story = {
  play: async ({ args }) => {
    await waitFor(() =>
      expect(screen.getByRole("button", { name: "Cancel" })).toHaveFocus()
    )
    await userEvent.keyboard("{Enter}")
    await expect(args.onCancel).toHaveBeenCalledOnce()
    await expect(args.onSample).not.toHaveBeenCalled()
  },
}
export const ExplicitRead: Story = {
  play: async ({ args }) => {
    const dialog = within(await screen.findByRole("dialog"))
    await waitFor(() =>
      expect(dialog.getByText(/This is a data read/)).toBeVisible()
    )
    await userEvent.click(dialog.getByRole("button", { name: "Read sample" }))
    await expect(args.onSample).toHaveBeenCalledOnce()
  },
}

export const ReadingLimits: Story = { args: { limits: null, loading: true } }
export const Unavailable: Story = {
  args: { limits: null },
  play: async () => {
    await expect(
      screen.getByRole("button", { name: "Read sample" })
    ).toBeDisabled()
  },
}
export const LimitsError: Story = {
  args: {
    limits: null,
    error: { message: "Example source session was closed.", retryable: false },
    onRetry: fn(),
  },
}
