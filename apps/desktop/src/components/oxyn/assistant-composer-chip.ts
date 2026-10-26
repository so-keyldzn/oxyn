// The look of a control in the composer's bottom bar: a flat chip — icon,
// label, small chevron — over a `SelectTrigger` that keeps its own keyboard,
// focus ring and ARIA (Base UI). Classes only: each control stays the
// component it was, with its name, its tooltip and its disabled reason.

/** Laid on a `SelectTrigger`; `cn` drops the border and box it brings. */
export const COMPOSER_CHIP =
  "h-7 min-w-0 gap-1 rounded-md border-transparent bg-transparent px-1.5 text-xs font-medium text-foreground/80 shadow-none hover:bg-muted hover:text-foreground aria-expanded:bg-muted aria-expanded:text-foreground dark:bg-transparent dark:hover:bg-muted data-[size=sm]:h-7 [&>svg:last-child]:hidden [&_svg:not([class*='size-'])]:size-3.5"

/**
 * The label of a chip: one line, cut with an ellipsis. Below the width the
 * composer gives it, a chip that says `hideable` keeps its icon alone — its
 * accessible name is the trigger's `aria-label`, never this text.
 */
export function chipLabel(hideable = false) {
  return hideable
    ? "hidden min-w-0 truncate @xs/composer:inline"
    : "min-w-0 truncate"
}

/**
 * A chip's list, as wide as its choices rather than as the chip: a chip can
 * be a mark alone, and a list that narrow would cut every name in it.
 */
export const CHIP_LIST = "w-auto max-w-80 min-w-(--anchor-width)"

/** The small chevron that closes a chip, in place of the select's own icon. */
export const CHIP_CHEVRON = "size-3 shrink-0 text-muted-foreground"
