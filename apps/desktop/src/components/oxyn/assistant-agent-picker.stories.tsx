import type { Meta, StoryObj } from "@storybook/react-vite"
import { expect, fn, userEvent, waitFor, within } from "storybook/test"

import { AssistantAgentPicker } from "./assistant-agent-picker"
import { SQL_AGENT_ID, SQL_ONLY } from "@/features/assistant/agent-options"
import type { AgentOption } from "@/lib/ipc/ai"

const schema: AgentOption = {
  id: "0199a3c0-0000-7000-8000-000000000002",
  name: "Schema",
  description: "Explains the schema and proposes changes.",
  origin: "shipped",
  error: null,
}
const user: AgentOption = {
  id: "user-analytics",
  name: "Analytics",
  description: "Helps explore reporting queries.",
  origin: "user",
  error: null,
}
const shipped = [...SQL_ONLY, schema]
const body = () => within(document.body)
async function closePopup() {
  await userEvent.keyboard("{Escape}")
  await waitFor(() =>
    expect(document.querySelector("[data-base-ui-focus-guard]")).toBeNull()
  )
}
async function openPopup() {
  await userEvent.click(body().getByRole("combobox", { name: "Agent role" }))
  const list = await body().findByRole("listbox", { name: "Agent role" })
  await waitFor(() => expect(list).toBeVisible())
  return within(list)
}

const meta = {
  title: "Oxyn/Assistant/AgentPicker",
  component: AssistantAgentPicker,
  args: {
    agents: shipped,
    value: null,
    onSelect: fn(),
  },
  decorators: [
    (Story) => (
      <div className="w-80 border p-3">
        <Story />
      </div>
    ),
  ],
} satisfies Meta<typeof AssistantAgentPicker>
export default meta
type Story = StoryObj<typeof meta>

export const Default: Story = {
  play: async ({ args }) => {
    await expect(body().getByRole("combobox")).toHaveTextContent("SQL")
    const list = await openPopup()
    await expect(list.getAllByRole("option")).toHaveLength(2)
    await userEvent.click(list.getByRole("option", { name: /^SQL/ }))
    await expect(args.onSelect).not.toHaveBeenCalled()
    await closePopup()
  },
}

export const WithUserAgent: Story = {
  args: { agents: [...shipped, user], value: user.id },
  play: async () => {
    await expect(body().getByRole("combobox")).toHaveTextContent(
      "AnalyticsUser"
    )
    const list = await openPopup()
    const options = list.getAllByRole("option")
    await expect(options.map((option) => option.textContent)).toEqual([
      expect.stringMatching(/^Schema/),
      expect.stringMatching(/^SQL/),
      expect.stringMatching(/^AnalyticsUser/),
    ])
    await closePopup()
  },
}

export const WithInvalidUserAgent: Story = {
  args: {
    agents: [
      ...shipped,
      { ...user, error: "analytics.md:7: unknown variable" },
    ],
  },
  play: async ({ args }) => {
    const list = await openPopup()
    const invalid = list.getByRole("option", { name: /Analytics/ })
    await expect(invalid).toHaveAttribute("aria-disabled", "true")
    await expect(invalid).toHaveTextContent("analytics.md:7: unknown variable")
    await expect(args.onSelect).not.toHaveBeenCalled()
    await closePopup()
  },
}

export const MissingAgentNotice: Story = {
  args: { value: user.id, missingAgent: { name: "Analytics" } },
  play: async () => {
    await expect(body().getByRole("combobox")).toHaveTextContent("SQL")
    await expect(body().getByRole("status")).toHaveTextContent(
      "Analytics is no longer available. This conversation continues with the SQL agent."
    )
  },
}

export const KeyboardNavigation: Story = {
  args: {
    agents: [
      ...shipped,
      { ...user, error: "analytics.md:7: unknown variable" },
    ],
  },
  play: async ({ args }) => {
    const trigger = body().getByRole("combobox", { name: "Agent role" })
    trigger.focus()
    await userEvent.keyboard("{Enter}")
    await waitFor(() => expect(body().getByRole("listbox")).toBeVisible())
    await userEvent.keyboard("{Home}{Enter}")
    await expect(args.onSelect).toHaveBeenCalledWith(schema.id)
    await closePopup()
    await expect(trigger).toHaveFocus()
  },
}

export const Loading: Story = { args: { loading: true } }
export const Empty: Story = { args: { agents: [] } }
export const SqlOnlyFallback: Story = {
  args: { agents: SQL_ONLY, unavailable: true },
  play: async () => {
    await expect(body().getByRole("combobox")).toHaveTextContent("SQL")
    const list = await openPopup()
    await expect(list.getAllByRole("option")).toHaveLength(1)
    await closePopup()
  },
}
export const RecordedAgentDuringListFailure: Story = {
  args: { agents: SQL_ONLY, value: user.id, unavailable: true },
  play: async ({ args }) => {
    await expect(body().getByRole("combobox")).toHaveTextContent(
      "Recorded agent"
    )
    const list = await openPopup()
    await userEvent.click(list.getByRole("option", { name: /^SQL/ }))
    await expect(args.onSelect).toHaveBeenCalledWith(SQL_AGENT_ID)
    await closePopup()
  },
}
export const Light: Story = { globals: { theme: "light" } }
