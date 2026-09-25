import type * as React from "react"
import { HugeiconsIcon } from "@hugeicons/react"
import { Cancel01Icon } from "@hugeicons/core-free-icons"
import type { IconSvgElement } from "@hugeicons/react"

import { Button } from "@/components/ui/button"
import { Kbd, KbdGroup } from "@/components/ui/kbd"
import { Tabs, TabsContent, TabsList, TabsTrigger } from "@/components/ui/tabs"
import {
  Tooltip,
  TooltipContent,
  TooltipTrigger,
} from "@/components/ui/tooltip"
import { actionKeys, ariaKeys } from "@/lib/actions/manifest"

/** One tab of the right column: the inspector, the assistant… */
export interface AsideItem {
  id: string
  label: string
  icon: IconSvgElement
  content: React.ReactNode
}

/**
 * The right column of the workspace. Its contents are mounted by whoever
 * composes the screen; every panel stays mounted when another tab is shown,
 * so a conversation or an inspection keeps its state.
 */
export function WorkspaceAside({
  items,
  active,
  onActiveChange,
  onClose,
}: {
  items: Array<AsideItem>
  active: string
  onActiveChange: (id: string) => void
  onClose: () => void
}) {
  return (
    <Tabs
      value={active}
      onValueChange={(value) => {
        if (typeof value === "string") onActiveChange(value)
      }}
      className="flex h-full min-h-0 flex-col gap-0 bg-sidebar"
    >
      <div className="flex h-10 shrink-0 items-center gap-1 border-b px-2">
        <TabsList variant="line">
          {items.map((item) => (
            <TabsTrigger key={item.id} value={item.id}>
              <HugeiconsIcon icon={item.icon} strokeWidth={2} />
              {item.label}
            </TabsTrigger>
          ))}
        </TabsList>
        {/* Shown like every other icon action of the frame: the name in the
            tooltip, the keys as `Kbd`, and the shortcut declared rather than
            spelled inside the name — a reader would otherwise say « Close
            side panel, left parenthesis, command… ». */}
        <Tooltip>
          <TooltipTrigger
            render={
              <Button
                size="icon-sm"
                variant="ghost"
                className="ml-auto"
                aria-label="Close side panel"
                aria-keyshortcuts={ariaKeys("view.sidePanel")}
                onClick={onClose}
              />
            }
          >
            <HugeiconsIcon icon={Cancel01Icon} strokeWidth={2} />
          </TooltipTrigger>
          <TooltipContent className="flex items-center gap-2">
            Close side panel
            <KbdGroup>
              {actionKeys("view.sidePanel").map((key) => (
                <Kbd key={key}>{key}</Kbd>
              ))}
            </KbdGroup>
          </TooltipContent>
        </Tooltip>
      </div>
      {items.map((item) => (
        <TabsContent
          key={item.id}
          value={item.id}
          keepMounted
          className="min-h-0 flex-1 overflow-hidden data-[hidden]:hidden"
        >
          {item.content}
        </TabsContent>
      ))}
    </Tabs>
  )
}
