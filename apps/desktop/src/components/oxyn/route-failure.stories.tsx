import type { Meta, StoryObj } from "@storybook/react-vite"
import { expect, fn, userEvent, waitFor } from "storybook/test"

import { RouteFailure } from "./route-failure"

const meta = {
  title: "Oxyn/RouteFailure",
  component: RouteFailure,
  decorators: [
    (Story) => (
      <div className="h-[480px] border">
        <Story />
      </div>
    ),
  ],
  args: { kind: "error", onReload: fn(), onBack: fn() },
} satisfies Meta<typeof RouteFailure>

export default meta
type Story = StoryObj<typeof meta>

/** A render failure: focus on the explanation, two ways out. */
export const RenderError: Story = {
  args: {
    message:
      'Failed to resolve import "@/components/oxyn/console-tabs" from "src/features/workspace/workspace-screen.tsx"',
  },
  play: async ({ canvas, args }) => {
    await waitFor(() =>
      expect(
        canvas.getByRole("heading", { name: "This screen stopped working" })
      ).toHaveFocus()
    )
    await userEvent.click(canvas.getByRole("button", { name: "Reload" }))
    await expect(args.onReload).toHaveBeenCalled()
    await userEvent.click(
      canvas.getByRole("button", { name: "Back to connections" })
    )
    await expect(args.onBack).toHaveBeenCalled()
  },
}

export const WithoutMessage: Story = {}

export const NotFound: Story = {
  args: { kind: "notFound", onReload: undefined },
  play: async ({ canvas }) => {
    await expect(canvas.queryByRole("button", { name: "Reload" })).toBeNull()
  },
}

/** A long, hostile message wraps and scrolls; it stays text. */
export const HostileMessage: Story = {
  args: {
    message: `<img src=x onerror=alert(1)> مخزن ‮gnp.exe ${'relation "users"; DROP TABLE audit; -- '.repeat(30)}`,
  },
  play: async ({ canvas }) => {
    await expect(canvas.getByText(/<img src=x/)).toBeVisible()
  },
}
