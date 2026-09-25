import * as React from "react"
import { QueryClientProvider } from "@tanstack/react-query"
import type { QueryClient } from "@tanstack/react-query"
import {
  HeadContent,
  Outlet,
  Scripts,
  createRootRouteWithContext,
} from "@tanstack/react-router"
import { useStore } from "@tanstack/react-store"
import { TanStackDevtools } from "@tanstack/react-devtools"
import { ReactQueryDevtoolsPanel } from "@tanstack/react-query-devtools"
import { TanStackRouterDevtoolsPanel } from "@tanstack/react-router-devtools"

import { Toaster } from "@/components/ui/toast"
import { AppMenubarHost } from "@/features/actions/app-menubar-host"
import { useActionRuntime } from "@/features/actions/use-action-runtime"
import { TooltipProvider } from "@/components/ui/tooltip"
import { subscribeToShutdown } from "@/features/consoles/draft-registry"
import { ProviderSettings } from "@/features/assistant/provider-settings"
import { openConnection, session } from "@/features/session"
import {
  SettingsDialog,
  openSettings,
} from "@/features/settings/settings-dialog"
import { useAppearance } from "@/features/settings/use-appearance"
import { useAsidePanels } from "@/features/workspace/aside-panels"
import { RouteError, RouteNotFound } from "@/features/workspace/route-failures"
import { WorkspaceHost } from "@/features/workspace/workspace-host"
import { platform } from "@/lib/actions/platform"
import { suppressBrowserDefaults } from "@/lib/browser-defaults"
import { subscribeToBackendEvents } from "@/lib/ipc/events"
import type { OpenConnection } from "@/lib/ipc/types"
import appCss from "../styles.css?url"

export const Route = createRootRouteWithContext<{ queryClient: QueryClient }>()(
  {
    head: () => ({
      meta: [
        { charSet: "utf-8" },
        { name: "viewport", content: "width=device-width, initial-scale=1" },
        { title: "Oxyn" },
      ],
      links: [{ rel: "stylesheet", href: appCss }],
    }),
    shellComponent: RootDocument,
    component: RootComponent,
    errorComponent: RouteError,
    notFoundComponent: RouteNotFound,
  }
)

// Declared once: providers and external agents are workspace-wide, not per
// connection (ADR-0023).
const AI_SECTIONS = [
  { id: "ai", label: "AI providers", content: <ProviderSettings /> },
]

const noSubscription = () => () => undefined

function RootComponent() {
  const { queryClient } = Route.useRouteContext()
  // The saved theme and reading density, applied to <html> (ADR-0013).
  useAppearance()
  const open = useStore(session, (state) => state.open)
  // Keyboard, focus zones and the macOS menu bar, for every screen (ADR-0041).
  useActionRuntime()
  // The shell is prerendered in Node, which knows no keyboard family: the
  // web menu bar appears once the window runs, never in the static HTML.
  const client = React.useSyncExternalStore(
    noSubscription,
    () => true,
    () => false
  )

  React.useEffect(() => {
    subscribeToBackendEvents()
    // Closing the window waits for the drafts this webview still holds.
    subscribeToShutdown()
  }, [])

  // Nothing of the webview shows: no page menu, reload, zoom or history
  // (ADR-0041 § 8).
  React.useEffect(() => suppressBrowserDefaults(window), [])

  return (
    <QueryClientProvider client={queryClient}>
      <TooltipProvider>
        <Toaster>
          <div className="flex h-full min-h-0 flex-col">
            {/* Windows and Linux: the first row, under the system title bar.
                macOS has its native bar, built in Rust (ADR-0041). */}
            {client && platform === "other" ? <AppMenubarHost /> : null}
            <div className="min-h-0 flex-1">
              <Outlet />
              {/* Above the routes: the start screen does not destroy it. */}
              <Workspace open={open} />
            </div>
          </div>
          {/* Once, for every screen: ⌘, opens it from the start screen too.
              An edit of the open connection hands back its new marking, which
              the workspace takes without closing a console (I-04). */}
          <SettingsDialog
            sections={AI_SECTIONS}
            openConnection={open}
            onOpenConnectionChanged={openConnection}
          />
        </Toaster>
      </TooltipProvider>
      <TanStackDevtools
        config={{ position: "bottom-left", hideUntilHover: true }}
        plugins={[
          { name: "TanStack Router", render: <TanStackRouterDevtoolsPanel /> },
          { name: "TanStack Query", render: <ReactQueryDevtoolsPanel /> },
        ]}
      />
    </QueryClientProvider>
  )
}

/**
 * The workspace and the panels of its right column.
 *
 * A component of its own because the panels ask the backend which providers
 * exist: the hook must run **inside** the query provider, not beside it.
 */
function Workspace({ open }: { open: OpenConnection | null }) {
  const aside = useAsidePanels(open)
  return <WorkspaceHost aside={aside} onOpenSettings={() => openSettings()} />
}

function RootDocument({ children }: { children: React.ReactNode }) {
  return (
    // Dark until the saved preference is read: a
    // database tool is used for hours, often next to a terminal.
    // `useAppearance` then owns the `dark` class and `data-density`.
    <html
      lang="en"
      className="dark h-full overflow-hidden"
      data-density="compact"
      suppressHydrationWarning
    >
      <head>
        <HeadContent />
      </head>
      <body className="h-full overflow-hidden">
        <div data-app-root>{children}</div>
        <Scripts />
      </body>
    </html>
  )
}
