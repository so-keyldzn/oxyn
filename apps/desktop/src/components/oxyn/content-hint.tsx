import type { ReactElement, ReactNode } from "react"

import {
  Tooltip,
  TooltipContent,
  TooltipTrigger,
} from "@/components/ui/tooltip"

/** Themed replacement for native titles without adding a layout wrapper or tab stop. */
export function ContentHint({
  content,
  children,
}: {
  content?: ReactNode
  children: ReactElement
}) {
  if (content == null || content === "") return children
  return (
    <Tooltip>
      <TooltipTrigger
        render={children}
        aria-description={typeof content === "string" ? content : undefined}
      />
      <TooltipContent className="wrap-anywhere whitespace-pre-wrap">
        {content}
      </TooltipContent>
    </Tooltip>
  )
}
