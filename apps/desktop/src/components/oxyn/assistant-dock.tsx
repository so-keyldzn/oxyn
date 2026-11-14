import * as React from "react"
import { HugeiconsIcon } from "@hugeicons/react"
import { Cancel01Icon, SparklesIcon } from "@hugeicons/core-free-icons"
import { usePanelRef } from "react-resizable-panels"

import { Button } from "@/components/ui/button"
import { ResizableHandle, ResizablePanel } from "@/components/ui/resizable"
import { cn } from "@/lib/utils"

export const ASSISTANT_PANEL_ID = "assistant-dock"
const INITIAL_WIDTH = 440

/** The assistant owns its column; collapsing it never destroys its draft. */
export function AssistantDock({
  children,
  open,
  registered,
  onOpenChange,
}: {
  children: React.ReactNode
  open: boolean
  registered: boolean
  onOpenChange: (open: boolean) => void
}) {
  const panel = usePanelRef()
  const width = React.useRef(INITIAL_WIDTH)
  React.useEffect(() => {
    const current = panel.current
    if (!registered || !current) return
    // Bounds change with visibility. Wait until the group has adopted them,
    // otherwise an opening can still be clamped by the previous zero maximum.
    const frame = requestAnimationFrame(() => {
      if (open) current.resize(width.current)
    })
    return () => cancelAnimationFrame(frame)
  }, [open, registered, panel])
  return (
    <>
      <ResizableHandle
        withHandle
        aria-label="Resize assistant panel"
        className={cn(!open && "hidden")}
        onKeyDownCapture={(event) => {
          if (event.key !== "Home") return
          event.preventDefault()
          panel.current?.resize(INITIAL_WIDTH)
        }}
      />
      <ResizablePanel
        id={ASSISTANT_PANEL_ID}
        panelRef={panel}
        defaultSize={open ? `${INITIAL_WIDTH}px` : "0px"}
        minSize={open ? "280px" : "0px"}
        maxSize={open ? "640px" : "0px"}
        groupResizeBehavior="preserve-pixel-size"
        onResize={(size) => {
          if (size.inPixels > 0 && open) width.current = size.inPixels
        }}
      >
        <section
          aria-label="Assistant panel"
          inert={!open}
          className={cn(
            "flex h-full min-h-0 flex-col bg-sidebar",
            !open && "hidden"
          )}
        >
          <header className="flex h-10 shrink-0 items-center gap-2 border-b px-3">
            <HugeiconsIcon
              icon={SparklesIcon}
              strokeWidth={2}
              className="size-4"
              aria-hidden
            />
            <h2 className="text-sm font-medium">Assistant</h2>
            <Button
              variant="ghost"
              size="icon-sm"
              className="ml-auto"
              aria-label="Close assistant panel"
              onClick={() => onOpenChange(false)}
            >
              <HugeiconsIcon icon={Cancel01Icon} strokeWidth={2} />
            </Button>
          </header>
          <div className="min-h-0 flex-1">{children}</div>
        </section>
      </ResizablePanel>
    </>
  )
}
