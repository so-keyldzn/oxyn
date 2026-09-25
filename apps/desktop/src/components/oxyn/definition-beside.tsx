import * as React from "react"
import { usePanelRef } from "react-resizable-panels"

import {
  ResizableHandle,
  ResizablePanel,
  ResizablePanelGroup,
} from "@/components/ui/resizable"

/** The definition panel's width, in pixels (docs/adr/0018). */
export const DEFINITION_WIDTH = { initial: 424, min: 320, max: 640 } as const

/**
 * The object's metadata with its definition beside it, in the wide layout.
 *
 * The width belongs to the caller, so that it outlives this panel: leaving
 * for Data and coming back, or opening another object, keeps it. Resizing
 * only moves the handle; it never asks for the definition again.
 */
export function DefinitionBeside({
  definition,
  width,
  onWidthChange,
  children,
}: {
  /** `null` draws the metadata alone, across the whole width. */
  definition: React.ReactNode
  width: number
  onWidthChange: (width: number) => void
  children: React.ReactNode
}) {
  const panel = usePanelRef()
  const widthRef = React.useRef(width)
  widthRef.current = width

  // Another object's panel was resized: this one follows, once it is drawn.
  React.useEffect(() => {
    const current = panel.current?.getSize().inPixels ?? 0
    if (current > 0 && Math.abs(current - width) >= 1)
      panel.current?.resize(width)
  }, [width, panel])

  return (
    <ResizablePanelGroup
      orientation="horizontal"
      className="min-h-0 flex-1"
      onLayoutChanged={(_, meta) => {
        // A drag or a key on the handle, not a window that changed size.
        const size = panel.current?.getSize().inPixels
        if (meta.isUserInteraction && size) onWidthChange(Math.round(size))
      }}
    >
      {/* Always the same panel: remounting it would drop the tab's scroll. */}
      <ResizablePanel id="object-metadata" minSize="40%">
        <div className="flex h-full min-h-0 flex-col">{children}</div>
      </ResizablePanel>
      {definition !== null ? (
        <>
          <ResizableHandle
            withHandle
            aria-label="Resize the definition panel"
            className="after:w-2"
            onKeyDownCapture={(event) => {
              // Home restores the initial width instead of the minimum. Taken
              // in the capture phase: the library listens on the handle itself
              // and gives way to an event already handled.
              if (event.key !== "Home") return
              event.preventDefault()
              panel.current?.resize(DEFINITION_WIDTH.initial)
              onWidthChange(DEFINITION_WIDTH.initial)
            }}
          />
          <ResizablePanel
            id="object-definition"
            panelRef={panel}
            defaultSize={width}
            minSize={DEFINITION_WIDTH.min}
            maxSize={DEFINITION_WIDTH.max}
            groupResizeBehavior="preserve-pixel-size"
            onResize={(size, _, previous) => {
              // Shown again after its tab was hidden: back to the width the
              // workspace holds, which may have changed meanwhile.
              if (previous?.inPixels === 0 && size.inPixels > 0)
                panel.current?.resize(widthRef.current)
            }}
          >
            <section
              aria-label="Definition"
              className="flex h-full min-h-0 flex-col"
            >
              {definition}
            </section>
          </ResizablePanel>
        </>
      ) : null}
    </ResizablePanelGroup>
  )
}
