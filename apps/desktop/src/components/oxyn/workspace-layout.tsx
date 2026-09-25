import * as React from "react"
import { HugeiconsIcon } from "@hugeicons/react"
import {
  Settings01Icon,
  SidebarRight01Icon,
  Unlink01Icon,
} from "@hugeicons/core-free-icons"
import { usePanelRef } from "react-resizable-panels"

import { EnvironmentBadge } from "@/components/oxyn/environment-badge"
import { ReadOnlyBadge } from "@/components/oxyn/read-only-badge"
import { Button } from "@/components/ui/button"
import { Kbd, KbdGroup } from "@/components/ui/kbd"
import {
  ResizableHandle,
  ResizablePanel,
  ResizablePanelGroup,
} from "@/components/ui/resizable"
import { Separator } from "@/components/ui/separator"
import {
  SidebarInset,
  SidebarProvider,
  SidebarTrigger,
} from "@/components/ui/sidebar"
import { Tabs } from "@/components/ui/tabs"
import { ToggleGroup, ToggleGroupItem } from "@/components/ui/toggle-group"
import {
  Tooltip,
  TooltipContent,
  TooltipTrigger,
} from "@/components/ui/tooltip"
import type { Environment } from "@/lib/ipc/types"
import { cn } from "@/lib/utils"

export type LeftView = "catalog" | "library"

/**
 * The right column's width in logical pixels (ADR-0013): 280 comes from
 * Figma and is what Home restores; the bounds are implementation guards,
 * the same the backend validates before saving.
 */
export const ASIDE_WIDTH = { initial: 280, min: 240, max: 480 } as const

function boundedAsideWidth(width: number) {
  return Math.min(ASIDE_WIDTH.max, Math.max(ASIDE_WIDTH.min, Math.round(width)))
}

function IconAction({
  label,
  keys,
  children,
  ...props
}: React.ComponentProps<typeof Button> & {
  label: string
  keys: Array<string>
}) {
  return (
    <Tooltip>
      <TooltipTrigger
        render={
          <Button
            size="icon-sm"
            variant="ghost"
            aria-label={label}
            {...props}
          />
        }
      >
        {children}
      </TooltipTrigger>
      <TooltipContent className="flex items-center gap-2">
        {label}
        <KbdGroup>
          {keys.map((key) => (
            <Kbd key={key}>{key}</Kbd>
          ))}
        </KbdGroup>
      </TooltipContent>
    </Tooltip>
  )
}

/**
 * The frame of a workspace (ADR-0011): left sidebar, a 48 px top bar that
 * always names the connection, its environment and READ ONLY, the tabs, the
 * work area, the right column and the status bar. Presentational: the screen
 * owns the state and gives every part by props.
 *
 * Below 1200 px (`compact`) the sidebar folds to its rail and the right column
 * overlays the work area instead of narrowing it. What the user chooses there
 * is not remembered: widening the window restores their wide-screen choice
 * (docs/UX-SPEC.md). The sidebar never goes off-canvas: the window is at
 * least 960 px wide.
 */
export function WorkspaceLayout({
  sidebar,
  sidebarOpen,
  onSidebarOpenChange,
  leftView,
  onLeftViewChange,
  connectionName,
  environment,
  readOnly,
  tabs,
  activeTab,
  onActiveTabChange,
  notice = null,
  children,
  aiEntry = null,
  aside = null,
  asideOpen = false,
  onAsideOpenChange,
  asideWidth = ASIDE_WIDTH.initial,
  onAsideWidthCommit,
  compact,
  onOpenSettings,
  onDisconnect,
  statusBar,
}: {
  sidebar: React.ReactNode
  /** The wide-screen preference. */
  sidebarOpen: boolean
  onSidebarOpenChange: (open: boolean) => void
  leftView: LeftView
  onLeftViewChange: (view: LeftView) => void
  connectionName: string
  environment: Environment
  readOnly: boolean
  /** `WorkspaceTabs`, rendered inside the `Tabs` root. */
  tabs: React.ReactNode
  activeTab: string | null
  onActiveTabChange: (key: string) => void
  /** What the workspace itself has to say — never a bound value. */
  notice?: string | null
  /** The tab panels (`TabsContent`), or the empty state. */
  children: React.ReactNode
  /** The `Ask AI` entry, absent when nothing is declared (docs/UX-SPEC.md). */
  aiEntry?: React.ReactNode
  /** The right column, kept mounted while closed. */
  aside?: React.ReactNode
  asideOpen?: boolean
  onAsideOpenChange?: (open: boolean) => void
  /** The saved wide-screen width of the right column, in pixels. */
  asideWidth?: number
  /**
   * The width the user settled on: once a drag is released or a key pressed,
   * never on every frame (docs/UX-SPEC.md).
   */
  onAsideWidthCommit?: (width: number) => void
  compact: boolean
  onOpenSettings?: () => void
  onDisconnect?: () => void
  statusBar: React.ReactNode
}) {
  // Only lives while compact: leaving compact drops it.
  const [compactOpen, setCompactOpen] = React.useState(false)
  React.useEffect(() => setCompactOpen(false), [compact])

  const asidePanel = usePanelRef()
  React.useEffect(() => {
    const panel = asidePanel.current
    if (compact || !panel) return
    if (asideOpen) panel.expand()
    else panel.collapse()
  }, [asideOpen, compact, asidePanel])

  // `defaultSize` only counts at mount: a width read or restored later
  // reaches the open column here.
  const width = boundedAsideWidth(asideWidth)
  React.useEffect(() => {
    const panel = asidePanel.current
    if (compact || !asideOpen || !panel || panel.isCollapsed()) return
    if (Math.round(panel.getSize().inPixels) !== width) panel.resize(width)
  }, [width, asideOpen, compact, asidePanel])

  // The layout settles once the pointer is released or a key has resized,
  // never per frame. The width is the panel's measured one, read on the next
  // frame: a key press has not redrawn the panel yet when the layout settles.
  // A collapse is closing the column, not a width to remember.
  const commitAsideWidth = (meta: { isUserInteraction: boolean }) => {
    if (!meta.isUserInteraction) return
    requestAnimationFrame(() => {
      const panel = asidePanel.current
      if (!panel || panel.isCollapsed()) return
      onAsideWidthCommit?.(boundedAsideWidth(panel.getSize().inPixels))
    })
  }

  // Home restores 280 px (docs/UX-SPEC.md), where the library would move the
  // handle to its end. Taken in the capture phase: the library listens on the
  // handle itself and skips an event already handled.
  const restoreAsideWidth = (event: React.KeyboardEvent) => {
    const panel = asidePanel.current
    if (event.key !== "Home" || !panel) return
    event.preventDefault()
    panel.resize(ASIDE_WIDTH.initial)
    onAsideWidthCommit?.(ASIDE_WIDTH.initial)
  }

  // Escape closes the overlaid column (docs/UX-SPEC.md). Watched on the whole
  // workspace, not on the column: it is opened from a button in the top bar,
  // so a handler inside the column only ever fires once focus is already
  // there. A dialog and a running statement both consume Escape first, which
  // `defaultPrevented` respects.
  const closeOverlaidAside = (event: React.KeyboardEvent) => {
    if (!compact || !asideOpen || aside === null) return
    if (event.key !== "Escape" || event.defaultPrevented) return
    onAsideOpenChange?.(false)
  }

  const main = (
    <div className="flex h-full min-h-0 flex-col">
      {notice ? (
        <p
          role="status"
          dir="auto"
          // Truncated on one line so it never steals height from the work
          // area; the whole sentence stays readable on hover.
          title={notice}
          className="shrink-0 truncate border-b px-3 py-1 text-[length:var(--reading-caption)] text-muted-foreground"
        >
          {notice}
        </p>
      ) : null}
      <div className="relative min-h-0 flex-1">{children}</div>
    </div>
  )

  return (
    <SidebarProvider
      open={compact ? compactOpen : sidebarOpen}
      onOpenChange={compact ? setCompactOpen : onSidebarOpenChange}
      onKeyDown={closeOverlaidAside}
      className="h-full min-h-0"
      style={
        {
          "--sidebar-width": "280px",
          "--sidebar-width-icon": "64px",
        } as React.CSSProperties
      }
    >
      {sidebar}
      <SidebarInset className="min-h-0 overflow-hidden">
        <Tabs
          value={activeTab}
          onValueChange={(value) => {
            if (typeof value === "string") onActiveTabChange(value)
          }}
          className="min-h-0 flex-1 gap-0"
        >
          <header
            data-tauri-drag-region
            data-slot="workspace-bar"
            className="flex h-12 shrink-0 items-center gap-2 border-b px-3"
          >
            <SidebarTrigger aria-label="Toggle sidebar" />
            <ToggleGroup
              value={[leftView]}
              onValueChange={(value: Array<unknown>) => {
                if (value[0] === "catalog" || value[0] === "library")
                  onLeftViewChange(value[0])
              }}
              variant="outline"
              size="sm"
              spacing={0}
              aria-label="Sidebar view"
            >
              <ToggleGroupItem value="catalog">Catalog</ToggleGroupItem>
              <ToggleGroupItem value="library">Library</ToggleGroupItem>
            </ToggleGroup>
            <Separator orientation="vertical" className="h-4" />
            <div className="flex max-w-64 min-w-0 shrink items-center gap-2">
              <span
                dir="auto"
                title={connectionName}
                className="min-w-0 truncate text-sm font-medium"
              >
                {connectionName}
              </span>
              <EnvironmentBadge environment={environment} />
              {readOnly ? <ReadOnlyBadge /> : null}
            </div>
            <div className="ml-2 min-w-0 flex-1">{tabs}</div>
            {aiEntry}
            {aside !== null && onAsideOpenChange ? (
              <IconAction
                label="Toggle side panel"
                keys={["⌘", "⌥", "B"]}
                variant={asideOpen ? "secondary" : "ghost"}
                aria-pressed={asideOpen}
                onClick={() => onAsideOpenChange(!asideOpen)}
              >
                <HugeiconsIcon icon={SidebarRight01Icon} strokeWidth={2} />
              </IconAction>
            ) : null}
            {onOpenSettings ? (
              <IconAction
                label="Settings"
                keys={["⌘", ","]}
                onClick={onOpenSettings}
              >
                <HugeiconsIcon icon={Settings01Icon} strokeWidth={2} />
              </IconAction>
            ) : null}
            {onDisconnect ? (
              <Tooltip>
                <TooltipTrigger
                  render={
                    <Button
                      size="icon-sm"
                      variant="ghost"
                      aria-label="Disconnect"
                      onClick={onDisconnect}
                    />
                  }
                >
                  <HugeiconsIcon icon={Unlink01Icon} strokeWidth={2} />
                </TooltipTrigger>
                <TooltipContent>
                  Disconnect — drafts are kept, sessions close
                </TooltipContent>
              </Tooltip>
            ) : null}
          </header>

          {/* Always the same group: remounting the main panel would drop every
              console's state when the side column opens or closes. */}
          <div className="relative flex min-h-0 flex-1">
            <ResizablePanelGroup
              orientation="horizontal"
              className="min-h-0 flex-1"
              onLayoutChanged={(_layout, meta) => commitAsideWidth(meta)}
            >
              {/* Unit-less sizes are percentages: the work area keeps at
                  least 40 % of the group, whatever the side column asks. */}
              <ResizablePanel id="work" minSize="40%">
                {main}
              </ResizablePanel>
              {aside !== null && !compact ? (
                <>
                  <ResizableHandle
                    withHandle
                    aria-label="Resize side panel"
                    onKeyDownCapture={restoreAsideWidth}
                    className={cn(!asideOpen && "hidden")}
                  />
                  <ResizablePanel
                    id="aside"
                    panelRef={asidePanel}
                    collapsible
                    collapsedSize="0px"
                    defaultSize={asideOpen ? `${width}px` : "0px"}
                    minSize={`${ASIDE_WIDTH.min}px`}
                    maxSize={`${ASIDE_WIDTH.max}px`}
                    onResize={(_size, _id, previous) => {
                      // Not on mount: the column is then drawn from the
                      // preference, and reporting it would save a width
                      // change as a choice (ADR-0013).
                      if (previous === undefined) return
                      if (asidePanel.current?.isCollapsed())
                        onAsideOpenChange?.(false)
                    }}
                  >
                    <div className={cn("h-full", !asideOpen && "hidden")}>
                      {aside}
                    </div>
                  </ResizablePanel>
                </>
              ) : null}
            </ResizablePanelGroup>
            {aside !== null && compact ? (
              // Below 1200 px the column overlays the work area
              // (docs/UX-SPEC.md). Escape is handled on the whole workspace,
              // not here: it is opened from a button in the bar, and a key
              // pressed there would never reach this subtree.
              <div
                data-slot="workspace-aside-overlay"
                className={cn(
                  "absolute inset-y-0 right-0 z-30 w-80 border-l bg-background shadow-lg",
                  !asideOpen && "hidden"
                )}
              >
                {aside}
              </div>
            ) : null}
          </div>
        </Tabs>
        {statusBar}
      </SidebarInset>
    </SidebarProvider>
  )
}
