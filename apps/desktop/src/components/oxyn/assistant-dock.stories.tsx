import * as React from "react"
import type { Meta, StoryObj } from "@storybook/react-vite"
import { expect, fn, userEvent, waitFor } from "storybook/test"

import { AssistantDock, ASSISTANT_PANEL_ID } from "./assistant-dock"
import { TextArea } from "./text-field"
import { Button } from "@/components/ui/button"
import { ResizablePanel, ResizablePanelGroup } from "@/components/ui/resizable"

const meta = {
  title: "Oxyn/AssistantDock",
  component: AssistantDock,
  args: { open: true, registered: true, onOpenChange: fn(), children: null },
  render: function Render() {
    const [open, setOpen] = React.useState(true)
    const [registered, setRegistered] = React.useState(false)
    return (
      <div className="flex h-[480px] w-[900px] flex-col">
        <Button onClick={() => setOpen(!open)}>Toggle assistant</Button>
        <ResizablePanelGroup
          orientation="horizontal"
          onLayoutChange={(layout) =>
            setRegistered(ASSISTANT_PANEL_ID in layout)
          }
        >
          <ResizablePanel id="work" minSize="40%">
            <div data-testid="work-content">Query workspace</div>
          </ResizablePanel>
          <AssistantDock
            open={open}
            registered={registered}
            onOpenChange={setOpen}
          >
            <TextArea
              aria-label="Question draft"
              defaultValue="Unsent question"
            />
          </AssistantDock>
        </ResizablePanelGroup>
      </div>
    )
  },
} satisfies Meta<typeof AssistantDock>
export default meta
type Story = StoryObj<typeof meta>

export const KeepsDraftWhenClosed: Story = {
  play: async ({ canvas }) => {
    const draft = canvas.getByRole("textbox", { name: "Question draft" })
    await userEvent.type(draft, " about orders")
    await userEvent.click(
      canvas.getByRole("button", { name: "Close assistant panel" })
    )
    await waitFor(() => expect(draft).not.toBeVisible())
    const work = canvas
      .getByTestId("work-content")
      .closest<HTMLElement>('[data-slot="resizable-panel"]')
    const group = work?.parentElement
    if (!work || !group) throw new Error("Missing panel layout")
    await waitFor(() =>
      expect(work.getBoundingClientRect().width).toBeGreaterThanOrEqual(
        group.getBoundingClientRect().width - 2
      )
    )
    await userEvent.click(
      canvas.getByRole("button", { name: "Toggle assistant" })
    )
    await waitFor(() => expect(draft).toBeVisible())
    await expect(draft).toHaveValue("Unsent question about orders")
  },
}
