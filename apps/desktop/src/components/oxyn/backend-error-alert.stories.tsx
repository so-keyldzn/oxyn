import type { Meta, StoryObj } from "@storybook/react-vite"
import { expect, fn, userEvent } from "storybook/test"

import { BackendErrorAlert } from "./backend-error-alert"

const meta = {
  title: "Oxyn/BackendErrorAlert",
  component: BackendErrorAlert,
  decorators: [
    (Story) => (
      <div className="max-w-xl p-6">
        <Story />
      </div>
    ),
  ],
  args: {
    title: "Connection failed",
    context: "Opening « billing »",
    error: {
      message:
        'FATAL:  password authentication failed for user "app" (SQLSTATE 28P01)',
      retryable: false,
    },
    nextStep: "Check the user and password, then connect again.",
    onRetry: fn(),
  },
} satisfies Meta<typeof BackendErrorAlert>

export default meta
type Story = StoryObj<typeof meta>

export const NotRetryable: Story = {
  play: async ({ canvas }) => {
    // No retry button for an error that would fail the same way.
    await expect(canvas.queryByRole("button", { name: "Try again" })).toBeNull()
    await expect(
      canvas.getByText("Check the user and password, then connect again.")
    ).toBeVisible()
  },
}

export const Retryable: Story = {
  args: {
    error: {
      message:
        'connection to server at "db.internal" (10.0.0.4), port 5432 failed: timeout expired',
      retryable: true,
    },
  },
  play: async ({ canvas, args }) => {
    await userEvent.click(canvas.getByRole("button", { name: "Try again" }))
    await expect(args.onRetry).toHaveBeenCalledOnce()
  },
}

export const BackendAbsent: Story = {
  args: {
    title: "Cannot list connections",
    context: undefined,
    error: {
      message:
        "The Oxyn backend is not available here (list_connections): open the desktop application.",
      retryable: false,
    },
    nextStep: undefined,
  },
}

export const LongHostileMessage: Story = {
  args: {
    error: {
      message: `ERROR:  relation "${"x".repeat(180)}<img src=x onerror=alert(1)>‮gnp.exe" does not exist`,
      retryable: false,
    },
  },
  play: async ({ canvasElement }) => {
    // Rendered as text: a message is hostile input, never markup.
    await expect(canvasElement.querySelector("img")).toBeNull()
  },
}

/**
 * A 180-character identifier in a 420 px panel. `break-words` let it set the
 * min-content width of the Alert's grid column, and the page grew to 1 391 px
 * at every width below 1 440. The message wraps inside the panel instead.
 */
export const LongMessageStaysInside: Story = {
  args: LongHostileMessage.args,
  decorators: [
    (Story) => (
      <div data-testid="room" className="w-[420px]">
        <Story />
      </div>
    ),
  ],
  play: async ({ canvas }) => {
    const room = canvas.getByTestId("room")
    await expect(room.scrollWidth).toBeLessThanOrEqual(room.clientWidth + 1)
    await expect(canvas.getByText(/does not exist/)).toBeVisible()
  },
}

export const Light: Story = { globals: { theme: "light" } }
