import * as React from "react"
import type { Meta, StoryObj } from "@storybook/react-vite"
import { expect, userEvent, within } from "storybook/test"

import { AssistantPlan } from "./assistant-plan"
import type { PlanEntry } from "./assistant-plan"
import { Button } from "@/components/ui/button"

const ENTRIES: Array<PlanEntry> = [
  {
    content: "Read the structure of invoices",
    priority: "high",
    status: "completed",
  },
  {
    content: "Read the structure of customers",
    priority: "medium",
    status: "completed",
  },
  {
    content: "Look for a join key between them",
    priority: "high",
    status: "inProgress",
  },
  {
    content: "Count the orphan invoices",
    priority: "low",
    status: "pending",
  },
]

const meta = {
  title: "Oxyn/Assistant/Plan",
  component: AssistantPlan,
  args: { entries: ENTRIES, running: false },
  decorators: [
    (Story) => (
      <div className="max-w-xl p-4">
        <Story />
      </div>
    ),
  ],
} satisfies Meta<typeof AssistantPlan>

export default meta
type Story = StoryObj<typeof meta>

/** The turn runs and no plan has arrived: a wait, not an empty frame. */
export const Planning: Story = {
  args: { entries: [], running: true },
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement)
    await expect(canvas.getByText("Planning the steps…")).toBeVisible()
  },
}

export const Running: Story = {
  args: { running: true },
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement)
    await expect(canvas.getByText("· 2 of 4 done")).toBeVisible()
    // The step under way is named as such, not only spun at.
    const running = canvasElement.querySelector('[data-status="inProgress"]')
    await expect(running).toHaveAttribute("aria-current", "step")
    // A plan describes intent: nothing here submits anything (I-07). The one
    // button is the fold.
    await expect(canvas.queryAllByRole("button")).toHaveLength(1)
  },
}

export const Finished: Story = {
  args: {
    entries: ENTRIES.map((entry) => ({
      ...entry,
      status: "completed" as const,
    })),
  },
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement)
    await expect(canvas.getByText("· 4 of 4 done")).toBeVisible()
  },
}

/** No plan at all draws nothing: there is nothing to say. */
export const Empty: Story = {
  args: { entries: [] },
  play: async ({ canvasElement }) => {
    await expect(canvasElement.textContent).toBe("")
  },
}

/**
 * The ordinary case, not an edge one: the agent sends its whole plan again,
 * and the client replaces. Nothing accumulates, and position — the only
 * identity a v1 entry has — may carry unrelated content on the next send.
 */
export const ReplacedByTheNextSend: Story = {
  render: function Harness() {
    const [sent, setSent] = React.useState(0)
    const next: Array<PlanEntry> = [
      {
        content: "Start again from the catalog",
        priority: "high",
        status: "completed",
      },
      {
        content: "Then look at the indexes",
        priority: "low",
        status: "pending",
      },
    ]
    return (
      <div className="flex flex-col gap-3">
        <AssistantPlan entries={sent === 0 ? ENTRIES : next} />
        {/* Story harness, not part of the component: it stands in for the
            agent sending its plan again. */}
        <Button size="sm" variant="outline" onClick={() => setSent(1)}>
          Send the next plan
        </Button>
      </div>
    )
  },
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement)
    await expect(canvas.getByText("· 2 of 4 done")).toBeVisible()
    await userEvent.click(
      canvas.getByRole("button", { name: "Send the next plan" })
    )
    // Replaced whole: the count follows the new list, and nothing of the old
    // one is left behind.
    await expect(canvas.getByText("· 1 of 2 done")).toBeVisible()
    await expect(canvas.getAllByRole("listitem")).toHaveLength(2)
    await expect(canvas.queryByText("Count the orphan invoices")).toBeNull()
    // Position 1 now carries other content and another status.
    const first = canvasElement.querySelectorAll(
      '[data-slot="assistant-plan-step"]'
    )[0]
    await expect(first).toHaveTextContent("Start again from the catalog")
    await expect(first).toHaveAttribute("data-status", "completed")
  },
}

/** Forty steps in the side panel: it scrolls down, never sideways. */
export const FortyStepsInANarrowPanel: Story = {
  args: {
    entries: Array.from({ length: 40 }, (_, index) => ({
      content: `Step ${index + 1}: read the structure of a table whose name is long enough to need two lines`,
      priority:
        index % 5 === 0
          ? ("high" as const)
          : index % 2 === 0
            ? ("medium" as const)
            : ("low" as const),
      status:
        index < 12
          ? ("completed" as const)
          : index === 12
            ? ("inProgress" as const)
            : ("pending" as const),
    })),
  },
  decorators: [
    (Story) => (
      <div className="w-[320px] border p-2">
        <Story />
      </div>
    ),
  ],
  play: async ({ canvasElement }) => {
    const list = canvasElement.querySelector("ol")
    await expect(list).not.toBeNull()
    // No horizontal overflow: a panel that scrolls sideways hides half of
    // every step behind a gesture nobody makes.
    await expect(list!.scrollWidth).toBeLessThanOrEqual(list!.clientWidth)
  },
}

/**
 * Step content is model output, so it is an hostile input: it is shown, never
 * interpreted (docs/SECURITY.md).
 */
export const HostileStepContent: Story = {
  args: {
    entries: [
      {
        content: '<img src=x onerror="alert(1)"> and <b>bold</b>',
        priority: "medium",
        status: "completed",
      },
      {
        content: "احسب عدد الفواتير غير المرتبطة بعميل",
        priority: "high",
        status: "inProgress",
      },
    ],
  },
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement)
    await expect(
      canvas.getByText('<img src=x onerror="alert(1)"> and <b>bold</b>')
    ).toBeVisible()
    await expect(canvasElement.querySelector("img")).toBeNull()
    await expect(canvasElement.querySelector("b")).toBeNull()
  },
}

/**
 * A status the protocol gains later must not blank the panel.
 *
 * The cast is the point of the story: it stands for a string that type
 * checking cannot catch, because it comes from a process we do not control.
 */
export const UnknownStatusFromALaterVersion: Story = {
  args: {
    entries: [
      ENTRIES[0]!,
      {
        content: "A step in a state this version does not know",
        priority: "medium",
        status: "blocked" as unknown as PlanEntry["status"],
      },
    ],
  },
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement)
    // It renders, it is not counted as done, and nothing throws.
    await expect(canvas.getByText("· 1 of 2 done")).toBeVisible()
    await expect(
      canvas.getByText("A step in a state this version does not know")
    ).toBeVisible()
  },
}

export const Folded: Story = {
  args: { defaultOpen: false },
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement)
    await expect(canvas.queryByRole("list")).toBeNull()
    await userEvent.click(canvas.getByRole("button", { name: /Plan/ }))
    await expect(canvas.getByRole("list")).toBeVisible()
  },
}

/**
 * The densest state in the light theme.
 *
 * Storybook renders dark by default, so without this story axe would never
 * check this component's text contrast in light — where the muted text, the
 * `High` marker and the success icon all sit on a different background.
 */
export const RunningInLight: Story = {
  args: { running: true },
  globals: { theme: "light" },
}
