import * as React from "react"
import { cn } from "cn"

// The parts of an `Item` rendered as a `button`. A button only admits phrasing
// content: the `div` and `p` of `ItemMedia`, `ItemContent`, `ItemTitle`,
// `ItemDescription` and `ItemActions` are invalid HTML inside one, which screen
// readers and some browsers render badly. These are the same parts as spans,
// with the same slots and classes — the classes set the display, so the layout
// does not change. `src/components/ui` stays as generated.

function ButtonItemMedia({
  className,
  ...props
}: React.ComponentProps<"span">) {
  return (
    <span
      data-slot="item-media"
      data-variant="icon"
      className={cn(
        "flex shrink-0 items-center justify-center gap-2 group-has-data-[slot=item-description]/item:translate-y-0.5 group-has-data-[slot=item-description]/item:self-start [&_svg]:pointer-events-none [&_svg:not([class*='size-'])]:size-4",
        className
      )}
      {...props}
    />
  )
}

function ButtonItemContent({
  className,
  ...props
}: React.ComponentProps<"span">) {
  return (
    <span
      data-slot="item-content"
      className={cn(
        "flex flex-1 flex-col gap-1 group-data-[size=xs]/item:gap-0 [&+[data-slot=item-content]]:flex-none",
        className
      )}
      {...props}
    />
  )
}

function ButtonItemTitle({
  className,
  ...props
}: React.ComponentProps<"span">) {
  return (
    <span
      data-slot="item-title"
      className={cn(
        "line-clamp-1 flex w-fit items-center gap-2 text-sm leading-snug font-medium underline-offset-4",
        className
      )}
      {...props}
    />
  )
}

function ButtonItemDescription({
  className,
  ...props
}: React.ComponentProps<"span">) {
  return (
    <span
      data-slot="item-description"
      className={cn(
        "line-clamp-2 text-left text-sm leading-normal font-normal text-muted-foreground group-data-[size=xs]/item:text-xs",
        className
      )}
      {...props}
    />
  )
}

function ButtonItemActions({
  className,
  ...props
}: React.ComponentProps<"span">) {
  return (
    <span
      data-slot="item-actions"
      className={cn("flex items-center gap-2", className)}
      {...props}
    />
  )
}

export {
  ButtonItemActions,
  ButtonItemContent,
  ButtonItemDescription,
  ButtonItemMedia,
  ButtonItemTitle,
}
