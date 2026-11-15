import type { Meta, StoryObj } from "@storybook/react-vite"
import { expect, fn, userEvent, waitFor, within } from "storybook/test"
import { SchemaMapLinkList } from "./schema-map-link-list"
import { SchemaMapView } from "./schema-map-view"
import { SchemaMapInspector } from "./schema-map-inspector"
import { mapFixture } from "./schema-map-fixtures"

const data = mapFixture()
const scope = { catalog: null, namespace: "sales" }
const meta = {
  title: "Oxyn/SchemaMapView",
  component: SchemaMapView,
  parameters: { layout: "fullscreen" },
  decorators: [
    (Story) => (
      <div className="h-[850px] p-3">
        <Story />
      </div>
    ),
  ],
  args: {
    connection: "Example development",
    scopes: [{ key: "sales", label: "sales", scope }],
    scopeKey: "sales",
    state: { data: null, loading: false, error: null },
    selected: null,
    definitionSupported: true,
    onScope: fn(),
    onLoad: fn(),
    onLoadScope: fn(),
    onCancel: fn(),
    onSelect: fn(),
    onOpen: fn(),
    onExpand: fn(),
    onInfer: fn(),
  },
} satisfies Meta<typeof SchemaMapView>
export default meta
type Story = StoryObj<typeof meta>
export const Initial: Story = {
  play: async ({ canvasElement, args }) => {
    const c = within(canvasElement)
    await expect(c.getByText("Choose a database or schema")).toBeVisible()
    await expect(args.onLoad).not.toHaveBeenCalled()
    await expect(
      c.queryByRole("button", { name: "Load selected scope" })
    ).toBeNull()
  },
}
export const Loading: Story = {
  args: { state: { data: null, loading: true, error: null } },
  play: async ({ canvasElement, args }) => {
    await userEvent.click(
      within(canvasElement).getByRole("button", { name: "Cancel read" })
    )
    await expect(args.onCancel).toHaveBeenCalled()
  },
}
export const Empty: Story = {
  args: {
    state: {
      data: {
        ...data,
        projection: {
          ...data.projection,
          objects: [],
          links: [],
          coverage: {
            ...data.projection.coverage,
            drawnObjects: 0,
            knownObjects: 0,
          },
        },
      },
      loading: false,
      error: null,
    },
  },
}
export const Ready: Story = {
  args: {
    state: { data, loading: false, error: null },
    selected: data.projection.objects[0]!.key,
  },
  play: async ({ canvasElement, args }) => {
    const c = within(canvasElement)
    await userEvent.click(c.getByRole("tab", { name: "Accessible list" }))
    await expect(c.getByRole("list", { name: "Mapped objects" })).toBeVisible()
    await expect(c.queryByText("0 objects not shown")).toBeNull()
    await expect(c.queryByText("0 structures not read")).toBeNull()
    await expect(
      c.getByText("Loaded scope only · this is not a complete database map.")
    ).toBeVisible()
    await userEvent.click(
      c.getByRole("button", { name: "How to read this map" })
    )
    await expect(
      c.getByText(/Incoming links cover loaded objects only/)
    ).toBeVisible()
    await expect(
      c.getAllByRole("button", { name: "Inspect orders" })
    ).toHaveLength(2)
    await userEvent.type(
      c.getByRole("textbox", { name: "Filter loaded objects" }),
      "archive"
    )
    await waitFor(() =>
      expect(c.getAllByRole("button", { name: "Inspect orders" })).toHaveLength(
        1
      )
    )
    await expect(args.onLoad).not.toHaveBeenCalled()
    await expect(args.onInfer).not.toHaveBeenCalled()
    await userEvent.click(c.getByRole("button", { name: "Open Structure" }))
    await expect(args.onOpen).toHaveBeenCalledWith(
      data.projection.objects[0],
      "structure"
    )
    await userEvent.click(c.getByRole("button", { name: "Expand neighbours" }))
    await expect(args.onExpand).toHaveBeenCalledWith(data.projection.objects[0])
  },
}
export const Partial: Story = {
  args: {
    state: {
      data: {
        ...data,
        issues: [
          {
            address: data.projection.objects[0]!.address,
            facet: "fields",
            kind: "failed",
          },
        ],
        projection: {
          ...data.projection,
          coverage: {
            ...data.projection.coverage,
            omittedObjects: 25,
            knownObjects: 27,
            omittedFields: 18,
            unresolvedLinks: 2,
          },
        },
      },
      loading: false,
      stale: true,
      error: null,
    },
  },
  play: async ({ canvasElement }) => {
    const c = within(canvasElement)
    await expect(c.getByText("25 objects not shown")).toBeVisible()
    await expect(c.getByText("18 fields not shown")).toBeVisible()
    await expect(c.getByText("2 links outside this map")).toBeVisible()
    await expect(c.queryByText("0 links not shown")).toBeNull()
    await expect(c.getByText(/Retained metadata may be stale/)).toBeVisible()
  },
}
export const Error: Story = {
  args: {
    state: {
      data: null,
      loading: false,
      error: { message: "The original session is closed.", retryable: false },
    },
  },
}
export const ChooseChild: Story = {
  args: {
    state: {
      data: { ...data, chooseNamespace: true },
      loading: false,
      error: null,
    },
  },
}
const sampled = mapFixture("collection")
sampled.projection.objects[0] = {
  ...sampled.projection.objects[0]!,
  fields: [],
  fieldsState: "unread",
  samplingRequired: true,
  keysSupported: false,
}
sampled.projection.sampleLimits = {
  maxDocuments: 1000,
  maxRetainedBytes: 8388608,
  maxDurationMs: 10000,
}
export const SamplingRequired: Story = {
  args: {
    state: { data: sampled, loading: false, error: null },
    selected: sampled.projection.objects[0].key,
    definitionSupported: false,
  },
  play: async ({ canvasElement, args }) => {
    const c = within(canvasElement)
    await expect(
      c.getByText("Field structure has not been read.")
    ).toBeVisible()
    await expect(args.onInfer).not.toHaveBeenCalled()
    await userEvent.click(c.getByRole("button", { name: "Infer fields…" }))
    await expect(args.onInfer).toHaveBeenCalledWith(
      sampled.projection.objects[0]
    )
  },
}
export const InspectorWithoutKeys: Story = {
  render: (args) => (
    <SchemaMapInspector
      object={{ ...data.projection.objects[0]!, keysSupported: false }}
      projection={{ ...data.projection, links: [] }}
      busy={false}
      definitionSupported={false}
      onOpen={args.onOpen}
      onExpand={args.onExpand}
    />
  ),
}

export const LinkList: Story = {
  render: () => <SchemaMapLinkList projection={data.projection} />,
}

const maximum = mapFixture()
maximum.projection.objects = Array.from({ length: 40 }, (_, index) => {
  const base = maximum.projection.objects[index % 2]!
  const key = `bounded-object-${index}`
  return {
    ...base,
    key,
    address: {
      catalog: "example",
      namespace: "public",
      relation: `object_${index}`,
    },
    fields: Array.from({ length: 12 }, (_field, position) => ({
      ...base.fields[0]!,
      key: `${key}:field-${position}`,
      name: `field_${position}`,
      path: [`field_${position}`],
    })),
  }
})
maximum.projection.links = Array.from({ length: 200 }, (_, index) => ({
  ...data.projection.links[0]!,
  key: `bounded-link-${index}`,
  from: maximum.projection.objects[index % 40]!.key,
  to: maximum.projection.objects[(index + 1 + Math.floor(index / 40)) % 40]!
    .key,
  target:
    maximum.projection.objects[(index + 1 + Math.floor(index / 40)) % 40]!
      .address,
  fields: ["field_0"],
  referencedFields: ["field_0"],
}))
maximum.projection.coverage = {
  ...maximum.projection.coverage,
  knownObjects: 40,
  drawnObjects: 40,
}
export const MaximumBound: Story = {
  args: { state: { data: maximum, loading: false, error: null } },
}

export const SelectedObject: Story = {
  args: { selectionLabel: "orders" },
  play: async ({ canvasElement, args }) => {
    const c = within(canvasElement)
    await expect(c.queryByRole("button", { name: "Load orders" })).toBeNull()
    await userEvent.click(c.getByRole("button", { name: "Show entire scope" }))
    await expect(args.onLoadScope).toHaveBeenCalled()
    await expect(args.onLoad).not.toHaveBeenCalled()
  },
}

export const Compact: Story = {
  // Geometry is measured after the real renderer module is available; this
  // story does not measure network/module startup time under parallel builds.
  beforeEach: async () => {
    await import("./erd-diagram")
  },
  parameters: {
    viewport: {
      options: {
        schemaCompact: {
          name: "Compact schema map",
          styles: { width: "860px", height: "720px" },
          type: "desktop",
        },
      },
    },
  },
  globals: { viewport: { value: "schemaCompact" } },
  decorators: [
    (Story) => (
      <div className="h-[620px] w-[800px] max-w-full">
        <Story />
      </div>
    ),
  ],
  args: {
    state: { data, loading: false, error: null },
    selected: data.projection.objects[0]!.key,
  },
  play: async ({ canvasElement }) => {
    const c = within(canvasElement)
    await userEvent.clear(
      c.getByRole("textbox", { name: "Filter loaded objects" })
    )
    await userEvent.click(c.getByRole("tab", { name: "Diagram" }))
    const diagram = await c.findByRole("region", {
      name: "Diagram of 2 objects",
    })
    const button = c.getByRole("button", {
      name: "Open Structure",
    })
    const card = button.closest('[data-slot="card"]')
    await waitFor(() =>
      expect(card?.getBoundingClientRect().top).toBeGreaterThanOrEqual(
        diagram.getBoundingClientRect().bottom
      )
    )
    const body = canvasElement.querySelector('[data-slot="schema-map-content"]')
    await expect(body?.scrollHeight).toBeGreaterThan(body?.clientHeight ?? 0)
    button.scrollIntoView({ block: "nearest" })
    await expect(button).toBeVisible()
  },
}
