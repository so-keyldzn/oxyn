import * as React from "react"
import { HugeiconsIcon } from "@hugeicons/react"
import type { IconSvgElement } from "@hugeicons/react"
import {
  Add01Icon,
  Cancel01Icon,
  CodeIcon,
  DashboardSquare01Icon,
  Layers01Icon,
  TableIcon,
  ViewIcon,
} from "@hugeicons/core-free-icons"

import { Button } from "@/components/ui/button"
import { Kbd, KbdGroup } from "@/components/ui/kbd"
import { Spinner } from "@/components/ui/spinner"
import { TabsList, TabsTrigger } from "@/components/ui/tabs"
import {
  Tooltip,
  TooltipContent,
  TooltipTrigger,
} from "@/components/ui/tooltip"
import { tabState, tabTitle } from "@/features/consoles/console-model"
import type { ConsoleTabInfo } from "@/features/consoles/console-model"
import { cn } from "@/lib/utils"

export type WorkspaceTabItem =
  | ({ kind: "console" } & ConsoleTabInfo)
  /** `objectKind` is the catalog kind: `table`, `view`, … */
  | { kind: "object"; key: string; title: string; objectKind: string }
  /** A retained result reopened from history. */
  | { kind: "result"; key: string; title: string }

const STATE_LABEL = {
  closing: "closing",
  saving: "saving",
  approval: "waiting for approval",
  running: "running",
  unsaved: "unsaved",
} as const

function iconOf(tab: WorkspaceTabItem): IconSvgElement {
  if (tab.kind === "console") return CodeIcon
  if (tab.kind === "result") return DashboardSquare01Icon
  if (tab.objectKind.includes("view")) return ViewIcon
  return tab.objectKind === "table" ? TableIcon : Layers01Icon
}

/** The value a tab and its panel share; consumers render panels with it. */
export function tabPanelValue(key: string) {
  return key
}

/**
 * The tabs of a workspace: objects, consoles, retained results, then « New
 * console ». Rendered inside a `Tabs` root owned by the screen, whose panels
 * carry the same values.
 *
 * Base UI tabs: arrow keys move between tabs, one tab stop, `tabpanel`s wired
 * by ARIA. A tab may own no button, so closing is Delete or Backspace on the
 * focused tab, ⌘W, a middle click, or the mouse-only cross.
 */
export function WorkspaceTabs({
  tabs,
  active = null,
  opening = false,
  onClose,
  onNewConsole,
  onCancelOpening,
}: {
  tabs: Array<WorkspaceTabItem>
  /** The tab the `Tabs` root shows; the strip scrolls to keep it in sight. */
  active?: string | null
  /** A console session is being opened; a second opening waits for it. */
  opening?: boolean
  onClose: (key: string) => void
  onNewConsole: () => void
  onCancelOpening: () => void
}) {
  const strip = React.useRef<HTMLDivElement>(null)
  // ⌘T with a full strip activates a tab the user cannot see: the new console
  // is off to the right, and nothing says the strip scrolls. Follow it.
  React.useEffect(() => {
    const tab = strip.current?.querySelector<HTMLElement>(
      '[data-slot="tabs-trigger"][data-active]'
    )
    tab?.scrollIntoView({ block: "nearest", inline: "nearest" })
  }, [active, tabs.length])

  return (
    <div className="flex min-w-0 items-center gap-1">
      <TabsList
        ref={strip}
        variant="line"
        aria-label="Workspace tabs"
        // The strip fills the 48 px bar rather than sitting at the 32 px the
        // list defaults to: `variant="line"` draws the active tab's underline
        // 5 px *below* the trigger, and `overflow-x-auto` — which makes the
        // vertical axis scrollable too — clips it out of sight otherwise.
        // The default carries the `horizontal` variant, so the override must.
        className="min-w-0 [scrollbar-width:none] justify-start overflow-x-auto group-data-horizontal/tabs:h-12"
      >
        {tabs.map((tab) => {
          const title = tab.kind === "console" ? tabTitle(tab) : tab.title
          const state = tab.kind === "console" ? tabState(tab) : null
          return (
            <TabsTrigger
              key={tab.key}
              value={tabPanelValue(tab.key)}
              aria-keyshortcuts="Delete"
              title={title}
              onKeyDown={(event) => {
                if (event.key === "Delete" || event.key === "Backspace") {
                  event.preventDefault()
                  onClose(tab.key)
                }
              }}
              onAuxClick={(event) => {
                if (event.button === 1) {
                  event.preventDefault()
                  onClose(tab.key)
                }
              }}
              className="group/tab h-8 max-w-56 flex-none gap-1.5 pr-1 text-[length:var(--reading-text)]"
            >
              <HugeiconsIcon icon={iconOf(tab)} strokeWidth={2} />
              <span dir="auto" className="min-w-0 truncate">
                {title}
              </span>
              {state ? (
                <span className="shrink-0 text-[length:var(--reading-caption)] font-normal text-muted-foreground">
                  · {STATE_LABEL[state]}
                </span>
              ) : null}
              <span
                aria-hidden
                data-slot="tab-close"
                onMouseDown={(event) => event.preventDefault()}
                onClick={(event) => {
                  event.stopPropagation()
                  onClose(tab.key)
                }}
                className={cn(
                  "flex size-5 shrink-0 cursor-pointer items-center justify-center rounded-sm opacity-60 hover:bg-muted hover:opacity-100",
                  "group-data-active/tab:opacity-100"
                )}
              >
                <HugeiconsIcon
                  icon={Cancel01Icon}
                  strokeWidth={2}
                  className="size-3"
                />
              </span>
            </TabsTrigger>
          )
        })}
      </TabsList>
      {opening ? (
        <Button size="sm" variant="ghost" onClick={onCancelOpening}>
          <Spinner data-icon="inline-start" />
          Cancel new console
        </Button>
      ) : (
        <Tooltip>
          <TooltipTrigger
            render={
              <Button
                size="icon-sm"
                variant="ghost"
                aria-label="New console"
                aria-keyshortcuts="Meta+T"
                onClick={onNewConsole}
              />
            }
          >
            <HugeiconsIcon icon={Add01Icon} strokeWidth={2} />
          </TooltipTrigger>
          <TooltipContent className="flex items-center gap-2">
            New console
            <KbdGroup>
              <Kbd>⌘</Kbd>
              <Kbd>T</Kbd>
            </KbdGroup>
          </TooltipContent>
        </Tooltip>
      )}
    </div>
  )
}
