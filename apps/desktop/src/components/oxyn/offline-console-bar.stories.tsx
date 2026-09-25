import type { Meta, StoryObj } from "@storybook/react-vite"
import { expect, fn, userEvent } from "storybook/test"

import { OfflineConsoleBar } from "./offline-console-bar"

const meta = {
  title: "Oxyn/OfflineConsoleBar",
  component: OfflineConsoleBar,
  decorators: [
    (Story) => (
      <div className="w-[720px] max-w-full border">
        <Story />
      </div>
    ),
  ],
  args: {
    connectionName: "billing-prod",
    sameConnection: true,
    attaching: false,
    onAttach: fn(),
    onCancel: fn(),
  },
} satisfies Meta<typeof OfflineConsoleBar>

export default meta
type Story = StoryObj<typeof meta>

/** On the connection the query was written for, the same editor resumes it. */
export const SameConnection: Story = {
  play: async ({ canvas, args }) => {
    await expect(
      canvas.getByText(/Choose a connection before running/)
    ).toBeVisible()
    await userEvent.click(
      canvas.getByRole("button", { name: "Connect to billing-prod" })
    )
    await expect(args.onAttach).toHaveBeenCalledOnce()
  },
}

/** Elsewhere it opens a copy: the wording says so before the click. */
export const OtherConnection: Story = {
  args: { sameConnection: false },
  play: async ({ canvas }) => {
    await expect(
      canvas.getByRole("button", { name: "Open a copy on billing-prod" })
    ).toBeVisible()
  },
}

/** Connecting can be cancelled; the console then stays offline. */
export const Attaching: Story = {
  args: { attaching: true },
  play: async ({ canvas, args }) => {
    await expect(
      canvas.queryByRole("button", { name: /Connect to/ })
    ).toBeNull()
    await userEvent.click(
      canvas.getByRole("button", { name: /Cancel connecting/ })
    )
    await expect(args.onCancel).toHaveBeenCalledOnce()
  },
}

export const HostileName: Story = {
  args: { connectionName: '"; DROP TABLE audit; -- مرحبا' },
  play: async ({ canvas }) => {
    await expect(
      canvas.getByRole("button", { name: /DROP TABLE audit/ })
    ).toBeVisible()
  },
}

export const Light: Story = { globals: { theme: "light" } }
