import type { Meta, StoryObj } from "@storybook/react-vite"
import { expect, fn, screen, userEvent, waitFor } from "storybook/test"
import { MetadataLoadingDialog } from "./metadata-loading-dialog"
import type { MetadataPreloadProgress } from "@/lib/ipc/metadata-loading"

const scope = { catalog: "Example", namespace: "public" }
const key = JSON.stringify(scope)
const meta = {
  title: "Oxyn/MetadataLoadingDialog",
  component: MetadataLoadingDialog,
  args: {
    connection: "Example development",
    policy: { version: 1, default: "minimal", overrides: [] },
    scopes: [{ key, label: "Example / public", scope }],
    scopeKey: key,
    saving: false,
    running: false,
    cancelling: false,
    report: null,
    error: null,
    onPolicy: fn(),
    onScope: fn(),
    onSave: fn(),
    onReload: fn(),
    onPreload: fn(),
    onCancel: fn(),
    onClose: fn(),
  },
} satisfies Meta<typeof MetadataLoadingDialog>
export default meta
type Story = StoryObj<typeof meta>
const report: MetadataPreloadProgress = {
  command: "example-command",
  connection: "example-source",
  session: "example-session",
  scope,
  level: "structure",
  policyRevision: 1,
  state: "partial",
  planned: 385,
  reads: 202,
  loaded: 201,
  cached: 183,
  failed: 1,
  unavailable: 0,
  samplingRequired: 0,
  knownRelations: 200,
  unreadRelations: 72,
  availableFacets: 383,
  staleFacets: 0,
  unreadFacets: 1,
  metadataBytes: 4096,
  elapsedMs: 120,
  stopped: null,
  next: {
    generation: "019a7c52-6aa0-7000-8000-000000000001",
    scope,
    policyRevision: 1,
    after: "orders",
  },
  failures: [{ relation: "orders", facet: "indexes" }],
  omittedFailures: 0,
}
export const Initial: Story = {
  play: async ({ args }) => {
    await waitFor(() =>
      expect(screen.getByRole("button", { name: "Close" })).toHaveFocus()
    )
    await userEvent.click(screen.getByRole("button", { name: "Structure" }))
    await expect(args.onPolicy).toHaveBeenCalledWith({
      version: 1,
      default: "structure",
      overrides: [],
    })
    await expect(args.onPreload).not.toHaveBeenCalled()
    await expect(
      screen.queryByRole("button", { name: "Read saved preferences" })
    ).toBeNull()
    await userEvent.click(
      screen.getByRole("button", { name: "Save preferences" })
    )
    await expect(args.onSave).toHaveBeenCalledOnce()
    await expect(args.onPreload).not.toHaveBeenCalled()
  },
}
export const Reading: Story = { args: { policy: null } }
export const Empty: Story = {
  args: { scopes: [], scopeKey: null },
  play: async () => {
    await userEvent.click(
      screen.getByRole("button", { name: "Advanced: optional preload" })
    )
    await expect(
      screen.getByRole("button", { name: "Preload scope" })
    ).toBeDisabled()
  },
}
export const InProgress: Story = {
  args: {
    running: true,
    report: {
      ...report,
      state: "running",
      next: null,
      availableFacets: null,
      staleFacets: null,
      unreadFacets: null,
    },
  },
  play: async ({ args }) => {
    await userEvent.click(
      await screen.findByRole("button", { name: "Cancel preload" })
    )
    await expect(args.onCancel).toHaveBeenCalledOnce()
    await expect(screen.getByRole("button", { name: "Close" })).toBeDisabled()
  },
}
export const Cancelling: Story = {
  args: {
    running: true,
    cancelling: true,
    report: {
      ...report,
      state: "cancelling",
      next: null,
      availableFacets: null,
      staleFacets: null,
      unreadFacets: null,
    },
  },
}
export const Partial: Story = {
  args: { report, policy: { version: 1, default: "structure", overrides: [] } },
  play: async ({ args }) => {
    await userEvent.click(
      await screen.findByRole("button", { name: "Load next page" })
    )
    await expect(args.onPreload).toHaveBeenCalledWith(true)
  },
}
export const LoadedEmpty: Story = {
  args: {
    report: {
      ...report,
      state: "complete",
      planned: 1,
      reads: 1,
      loaded: 1,
      cached: 0,
      failed: 0,
      knownRelations: 0,
      unreadRelations: 0,
      availableFacets: 0,
      staleFacets: 0,
      unreadFacets: 0,
      next: null,
      failures: [],
    },
  },
}
export const Error: Story = {
  args: {
    needsReload: true,
    error: {
      message: "Preferences changed in another window. Read them again.",
      retryable: false,
    },
  },
  play: async ({ args }) => {
    await expect(
      screen.getByRole("button", { name: "Save preferences" })
    ).toBeDisabled()
    await userEvent.click(
      screen.getByRole("button", { name: "Read saved preferences" })
    )
    await expect(args.onReload).toHaveBeenCalledOnce()
    await expect(screen.getByRole("button", { name: "Close" })).toBeEnabled()
  },
}

export const Unsaved: Story = {
  args: { dirty: true },
  play: async () => {
    await userEvent.click(
      screen.getByRole("button", { name: "Advanced: optional preload" })
    )
    await expect(
      screen.getByRole("button", { name: "Preload scope" })
    ).toBeDisabled()
    await expect(
      screen.getByRole("button", { name: "Save preferences" })
    ).toBeEnabled()
  },
}

export const Evicted: Story = {
  args: {
    report: {
      ...report,
      planned: 385,
      reads: 385,
      loaded: 385,
      cached: 0,
      failed: 0,
      knownRelations: 128,
      unreadRelations: 0,
      availableFacets: 200,
      staleFacets: 0,
      unreadFacets: 184,
      next: null,
      failures: [],
    },
  },
  play: async () => {
    await waitFor(() =>
      expect(screen.getByText(/184 unread\/evicted/)).toBeVisible()
    )
  },
}

export const OptionalPreloadStartsCollapsed: Story = {
  play: async ({ args }) => {
    await expect(
      screen.queryByRole("button", { name: "Preload scope" })
    ).toBeNull()
    await waitFor(() =>
      expect(screen.getByText(/No preparation is required/)).toBeVisible()
    )
    await userEvent.click(
      screen.getByRole("button", { name: "Advanced: optional preload" })
    )
    await expect(
      screen.getByRole("button", { name: "Preload scope" })
    ).toBeVisible()
    await expect(args.onPreload).not.toHaveBeenCalled()
  },
}
