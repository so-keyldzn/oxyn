import * as React from "react"
import type { Meta, StoryObj } from "@storybook/react-vite"
import { expect, fn, userEvent, waitFor, within } from "storybook/test"

import { DEFINITION_WIDTH, DefinitionBeside } from "./definition-beside"
import { FacetFrame } from "./facet-frame"
import { invoicesDetail } from "./fixtures"
import { fetched, invoicesDefinition } from "./metadata-fixtures"
import { DefinitionActions, RelationDefinition } from "./relation-definition"
import { RelationStructure } from "./relation-structure"
import { ASIDE_WIDTH } from "./workspace-layout"

/**
 * Structure and its definition as the default 1280 px window draws them: the
 * catalog and the Object inspector each take their initial 280 px, which
 * leaves 720 px to share between the two panels.
 */
function Harness({
  density,
  onRefresh,
  onCancel,
}: {
  density: "compact" | "comfortable"
  onRefresh: () => void
  onCancel: () => void
}) {
  const [width, setWidth] = React.useState<number>(DEFINITION_WIDTH.initial)
  return (
    <div data-density={density} className="flex h-[820px] w-[1280px]">
      <nav
        aria-label="Catalog"
        className="shrink-0 border-r"
        style={{ width: ASIDE_WIDTH.initial }}
      />
      <main className="flex min-w-0 flex-1 flex-col">
        <DefinitionBeside
          width={width}
          onWidthChange={setWidth}
          definition={
            <FacetFrame
              label="the definition"
              freshness={fetched}
              load={{ status: "idle" }}
              unsupported={null}
              hasValue
              empty={false}
              emptyText=""
              onRefresh={onRefresh}
              onCancel={onCancel}
              actions={
                <DefinitionActions
                  definition={invoicesDefinition}
                  stale={false}
                  onCopy={() => undefined}
                  onOpenInConsole={() => undefined}
                />
              }
            >
              <RelationDefinition
                definition={invoicesDefinition}
                stale={false}
              />
            </FacetFrame>
          }
        >
          <section aria-label="Structure" className="flex h-full flex-col">
            {/* A refresh in flight: Cancel is the action that must be reached. */}
            <FacetFrame
              label="columns"
              freshness={{ state: "invalidated" }}
              load={{ status: "loading" }}
              unsupported={null}
              hasValue
              empty={false}
              emptyText=""
              onRefresh={onRefresh}
              onCancel={onCancel}
            >
              <RelationStructure detail={invoicesDetail} />
            </FacetFrame>
          </section>
        </DefinitionBeside>
      </main>
      <aside
        aria-label="Object"
        className="shrink-0 border-l"
        style={{ width: ASIDE_WIDTH.initial }}
      />
    </div>
  )
}

const meta = {
  title: "Oxyn/DefinitionBeside",
  component: Harness,
  args: { density: "compact", onRefresh: fn(), onCancel: fn() },
} satisfies Meta<typeof Harness>

export default meta
type Story = StoryObj<typeof meta>

/**
 * The panel's header actions lie within it, none cut by its edge. The table
 * below may scroll sideways: that is the data, not an action.
 */
const ACTIONS = /^(Refresh|Cancel|Copy DDL|Open DDL in console)$/

async function expectActionsInside(panel: HTMLElement) {
  const bounds = panel.getBoundingClientRect()
  for (const button of within(panel).getAllByRole("button", {
    name: ACTIONS,
  })) {
    const box = button.getBoundingClientRect()
    await expect({
      name: button.textContent,
      inside: box.left >= bounds.left && box.right <= bounds.right + 0.5,
    }).toEqual({ name: button.textContent, inside: true })
  }
}

/**
 * The actions follow the width of their own panel, not the window's: with
 * the three panels open, Structure's Cancel and the definition's Refresh stay
 * whole, and still do after the definition panel is widened (issue #120).
 */
async function actionsStayReachable({
  canvas,
  args,
}: {
  canvas: ReturnType<typeof within>
  args: { onRefresh: () => void; onCancel: () => void }
}) {
  const structure = canvas.getByRole("region", { name: "Structure" })
  const definition = canvas.getByRole("region", { name: "Definition" })
  await expectActionsInside(structure)
  await expectActionsInside(definition)

  canvas.getByRole("separator", { name: "Resize the definition panel" }).focus()
  for (let step = 0; step < 12; step += 1)
    await userEvent.keyboard("{ArrowLeft}")
  await waitFor(() =>
    expect(definition.getBoundingClientRect().width).toBeGreaterThan(
      DEFINITION_WIDTH.initial
    )
  )
  await expectActionsInside(structure)
  await expectActionsInside(definition)

  await userEvent.click(
    within(structure).getByRole("button", { name: "Cancel" })
  )
  await expect(args.onCancel).toHaveBeenCalled()
  await userEvent.click(
    within(definition).getByRole("button", { name: "Refresh" })
  )
  await expect(args.onRefresh).toHaveBeenCalled()
}

export const DarkCompact: Story = {
  globals: { theme: "dark" },
  play: actionsStayReachable,
}

export const LightComfortable: Story = {
  globals: { theme: "light" },
  args: { density: "comfortable" },
  play: actionsStayReachable,
}
