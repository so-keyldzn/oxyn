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
import { ActionOverlays } from "@/features/actions/action-overlays"
import { AppMenubarHost } from "@/features/actions/app-menubar-host"
import { useActionRuntime } from "@/features/actions/use-action-runtime"
import { TooltipProvider } from "@/components/ui/tooltip"
import { subscribeToShutdown } from "@/features/consoles/draft-registry"
import { useFileDrops } from "@/features/file-drops/use-file-drops"
import { ProviderSettings } from "@/features/assistant/provider-settings"
import { ExitTransactionsHost } from "@/features/recovery/exit-transactions-host"
import { refreshConnection, session } from "@/features/session"
import {
  SettingsDialog,
  openSettings,
} from "@/features/settings/settings-dialog"
import { useAppearance } from "@/features/settings/use-appearance"
import { useResultFormatRefresh } from "@/features/settings/use-result-format-refresh"
import { RouteError, RouteNotFound } from "@/features/workspace/route-failures"
import { WorkspaceHost } from "@/features/workspace/workspace-host"
import { platform } from "@/lib/actions/platform"
import { suppressBrowserDefaults } from "@/lib/browser-defaults"
import { subscribeToBackendEvents } from "@/lib/ipc/events"
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
  // Result pages already held follow a saved change of cell format.
  useResultFormatRefresh(queryClient)
  const workspaces = useStore(session, (state) => state.workspaces)
  // Keyboard, focus zones and the macOS menu bar, for every screen (ADR-0041).
  useActionRuntime()
  // Files dropped from the system, classified by Rust (ADR-0041, point 9).
  useFileDrops()
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
              {/* Above the routes: the start screen does not destroy them.
                  Inside the query provider: each workspace's panels ask the
                  backend which providers exist. */}
              <WorkspaceHost onOpenSettings={() => openSettings()} />
            </div>
          </div>
          {/* Once, for every screen: ⌘, opens it from the start screen too.
              An edit of an open connection hands back its new marking, which
              its workspace takes, shown or hidden, without closing a console
              (I-04). */}
          <SettingsDialog
            sections={AI_SECTIONS}
            openConnections={workspaces}
            onOpenConnectionChanged={refreshConnection}
          />
          {/* An exit held by an open transaction asks here (ADR-0043). */}
          <ExitTransactionsHost />
          {/* ⌘K, ⌘P and ⌘/ from every screen, through the registry. */}
          <ActionOverlays />
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
