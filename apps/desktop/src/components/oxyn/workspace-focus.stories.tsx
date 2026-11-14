import * as React from "react"
import type { Meta, StoryObj } from "@storybook/react-vite"
import { expect, userEvent, waitFor, screen } from "storybook/test"
import type { EditorView } from "@codemirror/view"
import { InformationCircleIcon } from "@hugeicons/core-free-icons"
import { ActionOverlays } from "@/features/actions/action-overlays"
import { modKey } from "@/lib/actions/platform"
import { useActionSource } from "@/lib/actions/context"
import { Button } from "@/components/ui/button"
import {
  Dialog,
  DialogContent,
  DialogTitle,
  DialogDescription,
} from "@/components/ui/dialog"
import {
  Sidebar,
  SidebarContent,
  SidebarMenuButton,
} from "@/components/ui/sidebar"
import { TabsContent } from "@/components/ui/tabs"
import { WorkspaceLayout } from "./workspace-layout"
import { WorkspaceTabs } from "./workspace-tabs"
import { WorkspaceAside } from "./workspace-aside"
import { ConsoleView } from "./console-view"
import { SqlEditor } from "./sql-editor"
import { ResultPanel } from "./result-panel"
import { invoiceColumns, syntheticPages } from "./fixtures"

const nothing = () => undefined
function FocusWorkspace({
  compact = false,
  closed = false,
}: {
  compact?: boolean
  closed?: boolean
}) {
  const [text, setText] = React.useState("SELECT 1;")
  const editor = React.useRef<EditorView | null>(null)
  const [sidebar, setSidebar] = React.useState(!closed)
  const [aside, setAside] = React.useState(!closed)
  const [assistant, setAssistant] = React.useState(!closed)
  const [selection, setSelection] = React.useState("")
  const [review, setReview] = React.useState(false)
  useActionSource(
    "workspace",
    {
      activeTab: "console:example",
      tabCount: 1,
      consoleCount: 1,
      objectActive: false,
      hasAside: true,
      hasAssistant: true,
    },
    {
      openConsole: nothing,
      closeActiveTab: nothing,
      nextTab: nothing,
      previousTab: nothing,
      showCatalog: nothing,
      showLibrary: nothing,
      focusPreview: nothing,
      focusConsole: nothing,
      toggleSidebar: () => setSidebar((value) => !value),
      toggleAside: () => setAside((value) => !value),
      openAssistant: () => setAssistant(true),
      switchConnection: nothing,
    }
  )
  return (
    <div className="h-[640px] w-[1280px] overflow-hidden border">
      <WorkspaceLayout
        sidebar={
          <Sidebar collapsible="icon">
            <SidebarContent>
              <SidebarMenuButton aria-label="Catalog focus">
                Catalog
              </SidebarMenuButton>
            </SidebarContent>
          </Sidebar>
        }
        sidebarOpen={sidebar}
        onSidebarOpenChange={setSidebar}
        leftView="catalog"
        onLeftViewChange={nothing}
        connectionName="Example development"
        environment="development"
        readOnly
        activeTab="console:example"
        onActiveTabChange={nothing}
        compact={compact}
        statusBar={null}
        tabs={
          <WorkspaceTabs
            tabs={[
              {
                kind: "console",
                key: "console:example",
                title: "example.sql",
                fromAgent: false,
                activity: "idle",
                unsaved: true,
              },
            ]}
            onClose={nothing}
            onNewConsole={nothing}
            onCancelOpening={nothing}
          />
        }
        asideOpen={aside}
        onAsideOpenChange={setAside}
        aside={
          <WorkspaceAside
            items={[
              {
                id: "inspector",
                label: "Inspector",
                icon: InformationCircleIcon,
                content: <Button>Inspect cell</Button>,
              },
            ]}
            active="inspector"
            onActiveChange={nothing}
            onClose={() => setAside(false)}
          />
        }
        assistantOpen={assistant}
        onAssistantOpenChange={setAssistant}
        assistant={<Button>Assistant question</Button>}
      >
        <TabsContent value="console:example" className="min-h-0 flex-1">
          <ConsoleView
            notice={null}
            draftNotice="Unsaved"
            toolbar={
              <div className="flex gap-2 p-2">
                <Button
                  onClick={() => {
                    editor.current?.dispatch({
                      selection: { anchor: 0, head: 6 },
                    })
                    editor.current?.focus()
                  }}
                >
                  Select SQL text
                </Button>
                <Button
                  onClick={() => {
                    const view = editor.current
                    setSelection(
                      view
                        ? `${view.state.selection.main.anchor}:${view.state.selection.main.head} ${view.state.doc.toString()}`
                        : "unavailable"
                    )
                  }}
                >
                  Inspect selection
                </Button>
                <output aria-label="Editor selection snapshot">
                  {selection}
                </output>
                <Button onClick={() => setReview(true)}>Open review</Button>
              </div>
            }
            editor={
              <SqlEditor
                value={text}
                onChange={setText}
                driver="postgres"
                onEditor={(view) => {
                  editor.current = view
                }}
              />
            }
            results={
              <ResultPanel
                state={{
                  status: "populated",
                  result: "example-result",
                  columns: invoiceColumns,
                  rows: 7,
                  elapsedMs: 2,
                  truncated: false,
                  complete: true,
                  cancelled: false,
                }}
                fetchPage={syntheticPages(7, 0)}
              />
            }
          />
        </TabsContent>
      </WorkspaceLayout>
      <div hidden inert>
        <main data-focus-workspace>
          <section data-focus-region="editor">
            <Button>Hidden workspace editor</Button>
          </section>
        </main>
      </div>
      <ActionOverlays />
      <Dialog open={review} onOpenChange={setReview}>
        <DialogContent>
          <DialogTitle>Example review</DialogTitle>
          <DialogDescription>
            The workspace shortcuts must wait for this review.
          </DialogDescription>
          <Button autoFocus onClick={() => setReview(false)}>
            Cancel review
          </Button>
        </DialogContent>
      </Dialog>
    </div>
  )
}
const meta = {
  title: "Oxyn/WorkspaceFocus",
  component: FocusWorkspace,
} satisfies Meta<typeof FocusWorkspace>
export default meta
type Story = StoryObj<typeof meta>

async function editorCycle(areas: number) {
  await userEvent.click(
    await screen.findByRole("button", { name: "Select SQL text" })
  )
  await waitFor(() =>
    expect(document.activeElement?.classList.contains("cm-content")).toBe(true)
  )
  await userEvent.keyboard("{F6}")
  await expect(document.activeElement?.getAttribute("role")).toBe("grid")
  for (let index = 1; index < areas; index++) await userEvent.keyboard("{F6}")
  await waitFor(() =>
    expect(document.activeElement?.classList.contains("cm-content")).toBe(true)
  )
  await userEvent.keyboard("{Shift>}{F6}{/Shift}")
  await expect(document.activeElement?.getAttribute("role")).toBe("tab")
  await userEvent.keyboard("{F6}")
  await userEvent.click(
    screen.getByRole("button", { name: "Inspect selection" })
  )
  await expect(
    screen.getByLabelText("Editor selection snapshot")
  ).toHaveTextContent("0:6 SELECT 1;")
}
export const Wide: Story = { play: async () => editorCycle(6) }
export const Compact: Story = {
  args: { compact: true },
  play: async () => editorCycle(5),
}
export const ClosedPanels: Story = {
  args: { closed: true },
  play: async () => editorCycle(3),
}
export const ModalHoldsFocus: Story = {
  play: async () => {
    await userEvent.click(
      await screen.findByRole("button", { name: "Open review" })
    )
    const cancel = await screen.findByRole("button", { name: "Cancel review" })
    await waitFor(() => expect(cancel).toHaveFocus())
    await userEvent.keyboard("{F6}{Shift>}{F6}{/Shift}")
    await expect(cancel).toHaveFocus()
    await userEvent.click(cancel)
  },
}

export const PaletteReturnsToWorkspace: Story = {
  play: async () => {
    await userEvent.click(
      await screen.findByRole("button", { name: "Select SQL text" })
    )
    await userEvent.keyboard(`{${modKey}>}k{/${modKey}}`)
    const search = await screen.findByLabelText("Search actions")
    await userEvent.type(search, "Focus next area")
    await userEvent.click(
      await screen.findByRole("option", { name: /Focus next area/ })
    )
    await waitFor(() =>
      expect(document.activeElement?.getAttribute("role")).toBe("grid")
    )
    await userEvent.click(
      screen.getByRole("button", { name: "Inspect selection" })
    )
    await expect(
      screen.getByLabelText("Editor selection snapshot")
    ).toHaveTextContent("0:6 SELECT 1;")
  },
}
