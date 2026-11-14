import type { Meta, StoryObj } from "@storybook/react-vite"
import { expect, fn, userEvent } from "storybook/test"
import { MultiBaseExplorer } from "./multi-base-explorer"

const group = {
  id: "example-server",
  name: "Development server",
  expanded: true,
  seed: "Orders",
  members: [
    {
      id: "example-orders",
      name: "Orders",
      location: "localhost / orders",
      environment: "development" as const,
      readOnly: true,
      state: "active" as const,
      transaction: "Transaction open in a console",
    },
  ],
  discovery: {
    state: "ready" as const,
    databases: [
      { name: "orders", can_connect: true },
      { name: "audit", can_connect: false },
    ],
  },
}
const meta = {
  title: "Oxyn/MultiBaseExplorer",
  component: MultiBaseExplorer,
  decorators: [
    (Story) => (
      <div className="w-80">
        <Story />
      </div>
    ),
  ],
  args: {
    groups: [group],
    ungrouped: [],
    selected: null,
    loading: false,
    error: null,
    canGroup: true,
    onSelect: fn(),
    onToggle: fn(),
    onActivate: fn(),
    onCancel: fn(),
    onDiscover: fn(),
    onDatabase: fn(),
    onCreateGroup: fn(),
    onRemoveGroup: fn(),
    onRenameGroup: fn(),
    onDisconnectGroup: fn(),
    onDisconnect: fn(),
    onUnlink: fn(),
    onAttach: fn(),
    onRefresh: fn(),
  },
} satisfies Meta<typeof MultiBaseExplorer>
export default meta
type Story = StoryObj<typeof meta>
export const Loaded: Story = {
  play: async ({ canvas, args }) => {
    await userEvent.click(
      canvas.getByRole("button", { name: /Orders.*Saved destination/ })
    )
    await expect(args.onActivate).not.toHaveBeenCalled()
    await expect(args.onSelect).toHaveBeenCalledWith(
      "connection:example-orders"
    )
    await userEvent.keyboard("{Enter}")
    await expect(args.onActivate).toHaveBeenCalledWith("example-orders")
  },
}
export const Initial: Story = {
  args: {
    groups: [
      {
        ...group,
        expanded: false,
        discovery: { state: "initial", databases: [] },
      },
    ],
  },
  play: async ({ canvas, args }) => {
    await userEvent.click(
      canvas.getByRole("button", { name: "Expand Development server" })
    )
    await expect(args.onToggle).toHaveBeenCalledWith(group.id)
    await expect(args.onActivate).not.toHaveBeenCalled()
    await expect(args.onDiscover).not.toHaveBeenCalled()
  },
}
export const Empty: Story = {
  args: { groups: [], ungrouped: [], canGroup: false },
}
export const Loading: Story = {
  args: { loading: true, groups: [], canGroup: false },
}
export const Discovering: Story = {
  args: {
    groups: [{ ...group, discovery: { state: "loading", databases: [] } }],
  },
}
export const EmptyServer: Story = {
  args: {
    groups: [{ ...group, discovery: { state: "ready", databases: [] } }],
  },
}
export const DiscoveryError: Story = {
  args: {
    groups: [
      {
        ...group,
        discovery: {
          state: "error",
          databases: [],
          error: {
            message: "Example server refused catalog access.",
            retryable: false,
          },
        },
      },
    ],
  },
}
export const ReadError: Story = {
  args: {
    groups: [],
    error: {
      message: "Example workspace could not be read.",
      retryable: false,
    },
    canGroup: false,
  },
}
export const Opening: Story = {
  args: {
    groups: [
      { ...group, members: [{ ...group.members[0]!, state: "opening" }] },
    ],
  },
}
export const Cancelling: Story = {
  args: {
    groups: [
      { ...group, members: [{ ...group.members[0]!, state: "cancelling" }] },
    ],
  },
}
