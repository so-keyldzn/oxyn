import type * as React from "react"

import { ScrollArea, ScrollBar } from "@/components/ui/scroll-area"
import { TabsList } from "@/components/ui/tabs"
import { cn } from "@/lib/utils"

/**
 * A horizontal tab strip with an overlay scrollbar in every webview.
 * The line indicator extends below its trigger: its space is inside the
 * viewport, above the scrollbar, so neither axis creates a native scroll rail.
 */
export function ScrollableTabsList({
  ref,
  children,
  className,
  size = "default",
  ...props
}: Omit<React.ComponentProps<typeof TabsList>, "variant"> & {
  size?: "default" | "sm"
}) {
  return (
    <ScrollArea
      className={cn(
        "max-w-full min-w-0",
        size === "sm" ? "h-10" : "h-12",
        className
      )}
    >
      <TabsList
        {...props}
        ref={ref}
        variant="line"
        className={cn(
          "w-max min-w-full justify-start overflow-visible",
          size === "sm"
            ? "group-data-horizontal/tabs:h-9 [&_[data-slot=tabs-trigger]]:h-6"
            : "group-data-horizontal/tabs:h-11 [&_[data-slot=tabs-trigger]]:h-8"
        )}
      >
        {children}
      </TabsList>
      <ScrollBar orientation="horizontal" className="data-horizontal:h-1" />
    </ScrollArea>
  )
}
