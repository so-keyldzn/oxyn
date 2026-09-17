import { cn } from "cn"
import { HugeiconsIcon } from "@hugeicons/react"
import { Loading03Icon } from "@hugeicons/core-free-icons"

// `strokeWidth` is taken out of the SVG props: the DOM type allows a string,
// Hugeicons only a number, and the spinner's stroke is not a caller's choice.
function Spinner({
  className,
  strokeWidth: _strokeWidth,
  ...props
}: React.ComponentProps<"svg">) {
  return (
    <HugeiconsIcon
      icon={Loading03Icon}
      strokeWidth={2}
      data-slot="spinner"
      role="status"
      aria-label="Loading"
      className={cn("size-4 animate-spin", className)}
      {...props}
    />
  )
}

export { Spinner }
